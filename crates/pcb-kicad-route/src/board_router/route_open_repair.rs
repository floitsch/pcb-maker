// Copyright (C) 2026 Florian Loitsch.

//! Route mode's open repair: after the ladder, each net KiCad still finds
//! open is routed alone against all the copper kept, pours lowered as
//! the local repair lowers them (`open_repair::lower_for_local`), and
//! kept only where KiCad finds the board better (`repair_layout_opens`, the
//! layout's additive repair). The result then describes the board written
//! (`account`, also for the layout's repair: `account_open_repair`): the
//! nets KiCad no longer finds open are routed, the repair's copper is in
//! their totals, and what the router had left is in `nets[].open_repair`.

use super::*;
use crate::open_repair::{LayoutRepairBudget, repair_layout_opens};
use serde_json::Value;

/// The search work the repair may take when the ladder left less: route
/// mode's other repairs after the ladder (solid thermals, necks) count
/// their checks as no work either. One net's search is at most 20 s of
/// work (the local repair's own limit).
const SEARCH_FLOOR_SECONDS: f64 = 20.0;

/// What route mode's open repair did (`board-router.json`'s `open_repair`).
#[derive(Clone, Debug, Serialize)]
pub struct KiCadRouteOpenRepair {
    /// `accepted`, `no_improvement`, `skipped` or `failed`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// KiCad's unconnected items before and after.
    pub opens_before: usize,
    pub opens_after: usize,
    /// The nets whose copper KiCad accepted, and the objects added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nets: Vec<String>,
    pub added_copper: usize,
    pub native_checks: usize,
    pub search_work_seconds: f64,
}

/// What route mode's open repair did to a net whose copper KiCad accepted
/// (`board-router.json`'s `nets[].open_repair`). The net's `status`,
/// `unconnected_terminals`, `open`, `vias` and `length_mm` describe the
/// board written; this is what the router had left.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct KiCadNetOpenRepair {
    /// `closed`: KiCad finds no unconnected item on the net after the
    /// repair (the net is `routed`, without `open`); `improved`: it still
    /// finds some (`kicad_unconnected_items`), and the net's own count and
    /// `open` are the router's from before the repair. KiCad's items do
    /// not say which terminals are apart after it: an item names the two
    /// nearest objects of two parts of the net, a track as often as a pad,
    /// and never a part's other pads.
    pub status: &'static str,
    /// KiCad's unconnected items on the net after the repair.
    pub kicad_unconnected_items: usize,
    /// The pads those items name (`U1.3`, the first `OPEN_PADS_LISTED`):
    /// each is at an end of a connection still missing. Absent where the
    /// items name tracks, vias or zones only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kicad_unconnected_pads: Vec<String>,
    /// The router's own count and reasons before the repair (`open`'s
    /// form): what the repair closed, where `closed`.
    pub unconnected_terminals_before: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open_before: Vec<KiCadOpenTerminals>,
    /// The repair's copper on the net, counted in the net's `vias` and
    /// `length_mm` and in the result's.
    pub added_vias: usize,
    pub added_length_mm: f64,
}

/// A net whose copper KiCad accepted, as `account` takes it: its
/// connection name, KiCad's unconnected items on it after the repair with
/// the pads they name, and the vias and track length the repair added.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RepairedNet {
    pub name: String,
    pub still_open: usize,
    pub open_pads: Vec<String>,
    pub vias: usize,
    pub length_mm: f64,
}

/// The repaired nets `names` of the board `after` against `before` (the
/// board the ladder kept), with KiCad's report on `after`
/// (`after_report`): per net its tracks and vias on both boards and
/// KiCad's unconnected items left on it. An error where KiCad's items name
/// no single net (the counts stay the router's then).
pub(super) fn repaired_nets(
    before: &Expr,
    after: &Expr,
    after_report: &Value,
    names: &[String],
) -> Result<Vec<RepairedNet>, String> {
    let open = native_kept::open_nets(after, after_report)
        .ok_or("KiCad's unconnected items after the open repair name no single net each")?;
    let open_pads = open_pads(after, after_report);
    let copper = |pcb: &Expr, name: &str| -> Result<(usize, f64), String> {
        let codes = kicad_nets::net_codes(pcb);
        let net_names = NetNames::of(pcb);
        let (mut vias, mut length) = (0, 0.0);
        for item in pcb.children() {
            let head = item.head();
            if !matches!(head, Some("segment" | "arc" | "via")) {
                continue;
            }
            let on_net = kicad_nets::item_net(item, &codes)
                .is_some_and(|net| net == name || net_names.normalize(net) == name);
            if !on_net {
                continue;
            }
            match head {
                Some("segment") => {
                    let [x, y] = form_xy(item, "start")?;
                    let [to_x, to_y] = form_xy(item, "end")?;
                    length += (to_x - x).hypot(to_y - y);
                }
                Some("arc") => length += board_stats::track_arc_length(item)?,
                _ => vias += 1,
            }
        }
        Ok((vias, length))
    };
    let net_names = NetNames::of(after);
    names
        .iter()
        .map(|raw| {
            let name = net_names.normalize(raw);
            let (vias_before, length_before) = copper(before, name)?;
            let (vias_after, length_after) = copper(after, name)?;
            Ok(RepairedNet {
                name: name.to_string(),
                still_open: open.get(name).copied().unwrap_or(0),
                open_pads: open_pads.get(name).cloned().unwrap_or_default(),
                vias: vias_after.saturating_sub(vias_before),
                length_mm: (length_after - length_before).max(0.0),
            })
        })
        .collect()
}

/// The pads KiCad's unconnected items of `report` name on `pcb`, per net
/// (its connection name): `U1.3`, sorted, each once, the first
/// `OPEN_PADS_LISTED`.
fn open_pads(pcb: &Expr, report: &Value) -> BTreeMap<String, Vec<String>> {
    let mut labels = BTreeMap::new();
    for footprint in pcb.children() {
        if footprint.head() != Some("footprint") {
            continue;
        }
        let Some(reference) = kicad_nets::footprint_reference(footprint) else {
            continue;
        };
        for pad in footprint.children() {
            if pad.head() == Some("pad")
                && let Some(id) = crate::open_repair::item_id(pad)
                && let Some(number) = pad.children().get(1).and_then(Expr::atom)
            {
                labels.insert(id, format!("{reference}.{number}"));
            }
        }
    }
    let names = NetNames::of(pcb);
    let codes = kicad_nets::net_codes(pcb);
    let mut pads: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (net, items) in crate::open_repair::native_opens(pcb, report, &codes) {
        for item in items {
            if let Some(label) = item["uuid"]
                .as_str()
                .and_then(|id| labels.get(&id.to_ascii_lowercase()))
            {
                pads.entry(names.normalize(net).to_owned())
                    .or_default()
                    .insert(label.clone());
            }
        }
    }
    pads.into_iter()
        .map(|(net, pads)| (net, pads.into_iter().take(OPEN_PADS_LISTED).collect()))
        .collect()
}

/// The layout's and route mode's accounting of an accepted open repair:
/// `result` describes the board in `before`; the repair wrote the board in
/// `after` (with KiCad's report on it, `drc.json`) by adding copper to the
/// nets `accepted`. Afterwards `result`'s nets, counts and totals describe
/// the board in `after` (`account`). An error, with `result` untouched,
/// where the boards or the report cannot be read or KiCad's items name no
/// single net each.
pub fn account_open_repair(
    result: &mut KiCadBoardRouterResult,
    before: &Path,
    after: &Path,
    board_id: &str,
    accepted: &[String],
) -> Result<(), String> {
    let read = |directory: &Path| -> Result<Expr, String> {
        let path = directory.join(format!("{board_id}.kicad_pcb"));
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        parse(&source).map_err(|error| format!("failed to parse {}: {error}", path.display()))
    };
    let report: Value = serde_json::from_str(
        &fs::read_to_string(after.join("drc.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let repaired = repaired_nets(&read(before)?, &read(after)?, &report, accepted)?;
    account(result, &repaired);
    Ok(())
}

/// Makes `result` describe the board the accepted repair wrote: each of
/// `repaired` gets its copper added to its net's `vias` and `length_mm`
/// (the first entry of the name the router left open, else the first) and
/// a `KiCadNetOpenRepair`; where KiCad finds no unconnected item left on
/// the net, every entry of the name the router left open is `routed` with
/// no open terminal. The result's counts and totals follow its nets, as
/// the router summed them.
pub(super) fn account(result: &mut KiCadBoardRouterResult, repaired: &[RepairedNet]) {
    for net in repaired {
        let named: Vec<usize> = (0..result.nets.len())
            .filter(|index| result.nets[*index].connection == net.name)
            .collect();
        let Some(&carrier) = named
            .iter()
            .find(|index| result.nets[**index].unconnected_terminals > 0)
            .or(named.first())
        else {
            continue;
        };
        for &index in &named {
            let entry = &mut result.nets[index];
            let was_open = entry.unconnected_terminals > 0 || entry.status != "routed";
            if index != carrier && !(was_open && net.still_open == 0) {
                continue;
            }
            let (vias, length_mm) = if index == carrier {
                (net.vias, net.length_mm)
            } else {
                (0, 0.0)
            };
            let note = KiCadNetOpenRepair {
                status: if net.still_open == 0 {
                    "closed"
                } else {
                    "improved"
                },
                kicad_unconnected_items: net.still_open,
                kicad_unconnected_pads: net.open_pads.clone(),
                unconnected_terminals_before: entry.unconnected_terminals,
                open_before: if net.still_open == 0 {
                    std::mem::take(&mut entry.open)
                } else {
                    entry.open.clone()
                },
                added_vias: vias,
                added_length_mm: length_mm,
            };
            entry.vias += vias;
            entry.length_mm += length_mm;
            if net.still_open == 0 && entry.terminals >= 2 {
                entry.status = "routed".into();
                entry.unconnected_terminals = 0;
            }
            entry.open_repair = Some(note);
        }
    }
    let routable = result.nets.iter().filter(|net| net.terminals >= 2);
    result.routed_connections = routable.filter(|net| net.status == "routed").count();
    result.unconnected_terminals = result
        .nets
        .iter()
        .map(|net| net.unconnected_terminals)
        .sum();
    result.vias = result.nets.iter().map(|net| net.vias).sum();
    result.length_mm = result.nets.iter().map(|net| net.length_mm).sum();
}

/// Route mode's repair after its ladder, on the board kept in `directory`:
/// not in a layout's own ladders (`kicad_kept`: the layout repairs the
/// board it keeps, `board_layout/additive_repair.rs`), a provisional
/// reroute or a run without KiCad; skipped with `tuning.post_route_repairs`
/// off and past the wall guard. Charges its work to the ladder and notes it
/// in `result.open_repair`. Returns whether it ran.
pub(super) fn after_ladder(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    ladder: &mut Ladder<'_>,
) -> Result<bool, String> {
    if config.kicad_kept.is_some()
        || config.provisional_native_check_limit.is_some()
        || config.skip_native_verification
    {
        return Ok(false);
    }
    let repair = if ladder.past_deadline() && config.post_route_repairs_on() {
        result
            .native
            .as_ref()
            .filter(|native| native.selected_net_unconnected_items > 0)
            .map(|native| KiCadRouteOpenRepair::skipped(native, "wall_guard_expired"))
    } else {
        let left = (ladder.ladder_budget - ladder.work).max(0.0);
        repair_route_opens(result, directory, board_id, config, left)?
    };
    let Some(repair) = repair else {
        return Ok(false);
    };
    ladder.work += repair.search_work_seconds;
    ladder.charged.search_seconds += repair.search_work_seconds;
    ladder.charge_checks(repair.native_checks);
    eprintln!("{}", repair.line());
    result.open_repair = Some(repair);
    Ok(true)
}

/// Runs the repair on the board kept in `directory` (the run's staging
/// directory) when KiCad finds connections open there, and adopts the
/// repaired project in its place. With `tuning.post_route_repairs` off it
/// is skipped (`post_route_repairs_off`): no search, no KiCad check, the
/// board untouched. Returns the search work charged.
pub(super) fn repair_route_opens(
    result: &mut KiCadBoardRouterResult,
    directory: &Path,
    board_id: &str,
    config: &KiCadBoardRouterConfig,
    work_left: f64,
) -> Result<Option<KiCadRouteOpenRepair>, String> {
    let Some(before) = result
        .native
        .as_ref()
        .filter(|native| native.selected_net_unconnected_items > 0)
        .map(|native| native.selected_net_unconnected_items)
    else {
        return Ok(None);
    };
    // A repair from KiCad's report (docs/router.md, "Repairs from KiCad's
    // report").
    if !config.post_route_repairs_on() {
        return Ok(result
            .native
            .as_ref()
            .map(|native| KiCadRouteOpenRepair::skipped(native, "post_route_repairs_off")));
    }
    let mut repair = KiCadRouteOpenRepair {
        status: "skipped",
        reason: None,
        opens_before: before,
        opens_after: before,
        nets: Vec::new(),
        added_copper: 0,
        native_checks: 0,
        search_work_seconds: 0.0,
    };
    let mut budget = LayoutRepairBudget {
        router: config,
        available_seconds: work_left.max(SEARCH_FLOOR_SECONDS),
        // Route mode counts KiCad's checks as no work (`checks_charged`).
        check_seconds: 0.0,
        native_checks: 0,
        search_work_seconds: 0.0,
        branch_fill_reserve_checks: Some(0),
    };
    let trial = temporary_next_to(directory);
    let candidate = repair_layout_opens(directory, board_id, &trial, &mut budget);
    repair.native_checks = budget.native_checks;
    repair.search_work_seconds = budget.search_work_seconds;
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(error) => {
            let _ = fs::remove_dir_all(&trial);
            repair.status = "failed";
            repair.reason = Some(error);
            return Ok(Some(repair));
        }
    };
    if candidate.status != "improved" {
        let _ = fs::remove_dir_all(&trial);
        repair.status = "no_improvement";
        repair.reason = Some(
            candidate
                .note
                .or_else(|| candidate.trials.first().map(|trial| trial.status))
                .unwrap_or("no_eligible_native_open")
                .to_string(),
        );
        return Ok(Some(repair));
    }
    let adopted = (|| -> Result<VerificationReport, String> {
        let native = verification::record_verified_additive_board(
            &trial,
            board_id,
            candidate.schematic_parity_checked,
        )?;
        if native.selected_net_unconnected_items >= before {
            return Err("selected_native_open_count_did_not_decrease".into());
        }
        Ok(native)
    })();
    let native = match adopted {
        Ok(native) => native,
        Err(error) => {
            let _ = fs::remove_dir_all(&trial);
            repair.status = "no_improvement";
            repair.reason = Some(error);
            return Ok(Some(repair));
        }
    };
    let copper =
        board_stats::inspect_kicad_saved_copper(&trial.join(format!("{board_id}.kicad_pcb")))?;
    let accepted: Vec<String> = candidate
        .trials
        .iter()
        .filter(|trial| trial.status == "accepted")
        .map(|trial| trial.net.clone())
        .collect();
    let previous = temporary_next_to(directory);
    fs::rename(directory, &previous)
        .map_err(|error| format!("stage the routed board for the open repair: {error}"))?;
    if let Err(error) = fs::rename(&trial, directory) {
        fs::rename(&previous, directory).map_err(|restore| {
            format!("could not adopt the open repair: {error}; restore: {restore}")
        })?;
        let _ = fs::remove_dir_all(&trial);
        repair.status = "failed";
        repair.reason = Some(format!("adopt the open repair: {error}"));
        return Ok(Some(repair));
    }
    // What the repair did per net, from the board the ladder kept and the
    // one adopted.
    let accounted = account_open_repair(result, &previous, directory, board_id, &accepted);
    let _ = fs::remove_dir_all(previous);
    repair.status = "accepted";
    repair.opens_after = native.selected_net_unconnected_items;
    repair.added_copper = candidate.added_copper;
    repair.nets = accepted;
    // Not accounted: the nets' counts stay the router's from before the
    // repair, and the result says so.
    if let Err(error) = accounted {
        repair.reason = Some(format!("nets not accounted: {error}"));
        result.model_precedes_additive_repair = true;
    }
    result.native = Some(native);
    result.closed_reuse_drc = None;
    result.saved_copper = Some(copper);
    Ok(Some(repair))
}

impl KiCadRouteOpenRepair {
    pub(super) fn skipped(native: &VerificationReport, reason: &str) -> Self {
        Self {
            status: "skipped",
            reason: Some(reason.into()),
            opens_before: native.selected_net_unconnected_items,
            opens_after: native.selected_net_unconnected_items,
            nets: Vec::new(),
            added_copper: 0,
            native_checks: 0,
            search_work_seconds: 0.0,
        }
    }

    /// The log line.
    pub(super) fn line(&self) -> String {
        format!(
            "open repair: {}{}, KiCad's unconnected items {} -> {}{} ({} KiCad checks, {:.1} s of search work)",
            self.status,
            self.reason
                .as_ref()
                .map_or(String::new(), |reason| format!(" ({reason})")),
            self.opens_before,
            self.opens_after,
            if self.nets.is_empty() {
                String::new()
            } else {
                format!(
                    ", {} objects on {}",
                    self.added_copper,
                    self.nets.join(", ")
                )
            },
            self.native_checks,
            self.search_work_seconds
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(
        name: &str,
        terminals: usize,
        open: usize,
        vias: usize,
        length: f64,
    ) -> KiCadBoardRouterNet {
        KiCadBoardRouterNet {
            connection: name.into(),
            terminals,
            status: if open > 0 { "partial" } else { "routed" }.into(),
            unconnected_terminals: open,
            vias,
            length_mm: length,
            open: if open > 0 {
                vec![KiCadOpenTerminals {
                    reason: "no_path_hard:board".into(),
                    terminals: open,
                    pads: vec!["U10.10".into()],
                    walled: Vec::new(),
                    wall: None,
                    blocked_by: Vec::new(),
                }]
            } else {
                Vec::new()
            },
            repair: None,
            open_repair: None,
            copy: None,
            pads: Vec::new(),
        }
    }

    #[test]
    fn the_result_describes_the_board_the_repair_wrote() {
        // split72_right: 420 connections, VDD open at U10.10; the repair
        // adds a via and 2.2 mm and KiCad finds no item open.
        let mut result = KiCadBoardRouterResult {
            routable_connections: 3,
            routed_connections: 1,
            unconnected_terminals: 3,
            vias: 41,
            length_mm: 1766.0,
            nets: vec![
                net("VDD", 165, 1, 38, 1736.0),
                net("GND", 40, 2, 3, 20.0),
                net("SDA", 2, 0, 0, 10.0),
            ],
            ..Default::default()
        };
        let repaired = |name: &str, still_open, vias, length_mm| RepairedNet {
            name: name.into(),
            still_open,
            open_pads: if still_open > 0 {
                vec!["J2.b11".into()]
            } else {
                Vec::new()
            },
            vias,
            length_mm,
        };
        account(
            &mut result,
            &[repaired("VDD", 0, 1, 2.2), repaired("GND", 1, 0, 1.5)],
        );
        // VDD: closed by KiCad's count.
        let vdd = &result.nets[0];
        assert_eq!(
            (vdd.status.as_str(), vdd.unconnected_terminals),
            ("routed", 0)
        );
        assert!(vdd.open.is_empty());
        assert_eq!((vdd.vias, vdd.length_mm), (39, 1738.2));
        let note = vdd.open_repair.as_ref().unwrap();
        assert_eq!((note.status, note.kicad_unconnected_items), ("closed", 0));
        assert_eq!(note.unconnected_terminals_before, 1);
        assert_eq!(note.open_before[0].pads, ["U10.10"]);
        assert_eq!((note.added_vias, note.added_length_mm), (1, 2.2));
        // GND: copper kept, KiCad still finds an item: the router's count
        // and reasons stay, the copper is counted.
        let gnd = &result.nets[1];
        assert_eq!(
            (gnd.status.as_str(), gnd.unconnected_terminals),
            ("partial", 2)
        );
        assert_eq!(gnd.open.len(), 1);
        assert_eq!(gnd.length_mm, 21.5);
        let note = gnd.open_repair.as_ref().unwrap();
        assert_eq!((note.status, note.kicad_unconnected_items), ("improved", 1));
        // The pad KiCad's item names is in the note; `open` stays the
        // router's.
        assert_eq!(note.kicad_unconnected_pads, ["J2.b11"]);
        assert_eq!(gnd.open[0].pads, ["U10.10"]);
        assert!(
            result.nets[0]
                .open_repair
                .as_ref()
                .unwrap()
                .kicad_unconnected_pads
                .is_empty()
        );
        assert!(result.nets[2].open_repair.is_none());
        // The totals follow the nets.
        assert_eq!(
            (result.routable_connections, result.routed_connections),
            (3, 2)
        );
        assert_eq!(result.unconnected_terminals, 2);
        assert_eq!(result.vias, 42);
        assert!((result.length_mm - 1769.7).abs() < 1.0e-9);
        let json = serde_json::to_value(&result.nets[0]).unwrap();
        assert!(json.get("open").is_none());
        assert_eq!(json["open_repair"]["status"], "closed");
    }

    #[test]
    fn a_repaired_net_is_measured_on_the_two_boards_and_in_kicads_report() {
        let pcb = |extra: &str| {
            parse(&format!(
                r#"(kicad_pcb (version 20241229)
                  (footprint "t" (layer "F.Cu") (at 3 5) (uuid "f1")
                    (property "Reference" "J1" (at 0 0) (layer "F.SilkS"))
                    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/LINK") (uuid "p1")))
                  (footprint "t" (layer "F.Cu") (at 16 5) (uuid "f2")
                    (property "Reference" "J2" (at 0 0) (layer "F.SilkS"))
                    (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "/LINK") (uuid "p2")))
                  (segment (start 3 5) (end 5 5) (width 0.2) (layer "F.Cu") (net "/LINK") (uuid "s1"))
                  (segment (start 0 0) (end 9 0) (width 0.2) (layer "F.Cu") (net "OTHER") (uuid "s2"))
                  {extra})"#
            ))
            .unwrap()
        };
        let before = pcb("");
        let after = pcb(
            r#"(segment (start 5 5) (end 16 5) (width 0.2) (layer "F.Cu") (net "/LINK") (uuid "s3"))
               (via (at 5 5) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net "/LINK") (uuid "v1"))"#,
        );
        let report = serde_json::json!({"kicad_version": "10.0.7", "unconnected_items": []});
        let repaired = repaired_nets(&before, &after, &report, &["/LINK".to_string()]).unwrap();
        assert_eq!(
            repaired,
            [RepairedNet {
                name: "LINK".into(),
                still_open: 0,
                open_pads: Vec::new(),
                vias: 1,
                length_mm: 11.0,
            }]
        );
        // KiCad still finds the pads apart: the net is not closed.
        let report = serde_json::json!({"kicad_version": "10.0.7", "unconnected_items": [
            {"items": [{"uuid": "p1"}, {"uuid": "p2"}]}]});
        let repaired = repaired_nets(&before, &after, &report, &["/LINK".to_string()]).unwrap();
        assert_eq!(repaired[0].still_open, 1);
        assert_eq!(repaired[0].open_pads, ["J1.1", "J2.1"]);
        // An item between a pad and a track names the pad alone.
        let report = serde_json::json!({"kicad_version": "10.0.7", "unconnected_items": [
            {"items": [{"uuid": "s3"}, {"uuid": "P2"}]}]});
        let repaired = repaired_nets(&before, &after, &report, &["/LINK".to_string()]).unwrap();
        assert_eq!(
            (repaired[0].still_open, &repaired[0].open_pads[..]),
            (1, &["J2.1".to_string()][..])
        );
    }

    #[test]
    fn an_open_net_is_routed_alone_and_adopted_with_kicads_evidence() {
        if !std::process::Command::new(native_report::kicad_cli())
            .arg("version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }
        let root = temporary_next_to(&std::env::temp_dir().join("route-open-repair"));
        let directory = root.join("route");
        fs::create_dir_all(&directory).unwrap();
        let pcb = r#"(kicad_pcb (version 20241229) (generator "pcbnew")
          (general (thickness 1.6)) (paper "A4")
          (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
          (gr_rect (start 0 0) (end 20 10) (stroke (width 0.05) (type default)) (layer "Edge.Cuts"))
          (footprint "test" (layer "F.Cu") (at 3 5)
            (uuid "bf2ff1e8-13dd-462e-9898-234e65f5ff01")
            (property "Reference" "J1" (at 0 -2) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "LINK")
              (uuid "1b0aa000-13dd-462e-9898-234e65f5ff01")))
          (footprint "test" (layer "F.Cu") (at 16 5)
            (uuid "bf2ff1e8-13dd-462e-9898-234e65f5ff02")
            (property "Reference" "J2" (at 0 -2) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "LINK")
              (uuid "1b0aa000-13dd-462e-9898-234e65f5ff02"))))"#;
        fs::write(directory.join("board.kicad_pcb"), pcb).unwrap();
        fs::write(directory.join("board.kicad_pro"), "{}\n").unwrap();
        let config = crate::resolve_project_rules(
            &directory.join("board.kicad_pro"),
            &directory.join("board.kicad_pcb"),
        )
        .unwrap();
        let native = verify_materialized_rung(&directory, "board").unwrap();
        assert_eq!(native.selected_net_unconnected_items, 1);
        let mut result = KiCadBoardRouterResult {
            native: Some(native),
            routable_connections: 1,
            unconnected_terminals: 1,
            nets: vec![net("LINK", 2, 1, 0, 0.0)],
            ..Default::default()
        };
        // With `tuning.post_route_repairs` false the repair is skipped with
        // its reason: no search, no KiCad check, the board untouched.
        let mut off = config.clone();
        off.tuning.post_route_repairs = Some(false);
        let skipped = repair_route_opens(&mut result, &directory, "board", &off, 0.0)
            .unwrap()
            .unwrap();
        assert_eq!(skipped.status, "skipped");
        assert_eq!(skipped.reason.as_deref(), Some("post_route_repairs_off"));
        assert_eq!((skipped.opens_before, skipped.opens_after), (1, 1));
        assert_eq!(
            (skipped.native_checks, skipped.search_work_seconds),
            (0, 0.0)
        );
        assert!(!result.model_precedes_additive_repair);
        assert!(result.saved_copper.is_none());
        assert_eq!(
            fs::read_to_string(directory.join("board.kicad_pcb")).unwrap(),
            pcb
        );
        assert!(!directory.join("open-repair.json").exists());
        // No work left: the floor still searches.
        let repair = repair_route_opens(&mut result, &directory, "board", &config, 0.0)
            .unwrap()
            .unwrap();
        assert_eq!(repair.status, "accepted", "{repair:?}");
        assert_eq!((repair.opens_before, repair.opens_after), (1, 0));
        assert_eq!(repair.nets, ["LINK"]);
        assert!(repair.native_checks >= 2);
        assert!(repair.search_work_seconds > 0.0);
        // Accounted: nothing of the result precedes the repair's copper.
        assert!(!result.model_precedes_additive_repair);
        assert!(result.native.as_ref().unwrap().complete);
        assert!(result.saved_copper.as_ref().unwrap().segments > 0);
        assert!(directory.join("open-repair.json").is_file());
        // The router's own counts describe the board written.
        assert_eq!(
            (result.routed_connections, result.unconnected_terminals),
            (1, 0)
        );
        let link = &result.nets[0];
        assert_eq!(link.status, "routed");
        assert!(link.open.is_empty() && link.length_mm > 12.0, "{link:?}");
        assert_eq!(link.open_repair.as_ref().unwrap().status, "closed");
        assert_eq!(result.length_mm, link.length_mm);
        let disk = fs::read_to_string(directory.join("board.kicad_pcb")).unwrap();
        assert!(disk.starts_with(pcb.strip_suffix(')').unwrap()));
        // A board KiCad finds connected has nothing to repair.
        assert!(
            repair_route_opens(&mut result, &directory, "board", &config, 0.0)
                .unwrap()
                .is_none()
        );
        fs::remove_dir_all(root).unwrap();
    }
}

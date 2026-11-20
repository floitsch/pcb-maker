// Copyright (C) 2026 Florian Loitsch.

//! Additive repair of two unstarted conductors using the ordinary pair router.

use super::*;
use pcb_router as core;

pub(super) fn propose(
    pcb: &Expr,
    path: &Path,
    report: &Value,
    selected: &BTreeSet<String>,
) -> Result<Option<([String; 2], local::Proposal)>, String> {
    let codes = net_codes(pcb);
    let opens = native_opens(pcb, report, &codes);
    let Some(nets) = selected.iter().find_map(|net| {
        let (mate, _, positive) = project_rules::diff_pair_of(net)?;
        if !positive || !selected.contains(&mate) {
            return None;
        }
        let names = [net.clone(), mate];
        eligible(pcb, &codes, &opens, [&names[0], &names[1]]).then_some(names)
    }) else {
        return Ok(None);
    };
    let rules = project_rules::resolve_project_rules(&path.with_extension("kicad_pro"), path)?;
    let choice = rules.diff_pairs_choice();
    if (choice.default == "explicit" && !choice.on)
        || project_rules::LocalPairRequirements::read(path).is_none()
    {
        return Ok(None);
    }
    let mut pair = describe(pcb, &codes, &opens, [&nets[0], &nets[1]]);
    let search = route_pair(pcb, rules, [&nets[0], &nets[1]], &mut pair)?;
    let mut proposal = local::Proposal::skipped(pair.reason.unwrap_or("no_coupled_local_path"));
    if let Some((objects, geometry)) = search {
        proposal = local::Proposal::candidate(objects);
        pair.geometry = Some(geometry);
    } else {
        let reason = pair.reason.unwrap_or("no_coupled_local_path");
        pair.reason = Some(reason);
        pair.next_action = Some(match reason {
            "coupled_endpoints_not_recognized" if pair.unmatched_end.is_some() => {
                "If the mechanical brief permits, compare a placement with the listed pair pads closer together. Keep the pair's original rules and check both conductors in KiCad."
            }
            "coupled_endpoints_not_recognized" => {
                "Review the listed pads as two physical pair ends before comparing a coupled reroute with the original rules."
            }
            "coupled_launch_unavailable" => {
                "Review the copper and pad clearances around the listed pair ends before comparing a coupled reroute."
            }
            "coupled_local_search_limit" => {
                "Compare a coupled route from the matching placed board with the original rules."
            }
            _ => {
                "Review the corridor between the pair ends before comparing a coupled reroute with the original rules."
            }
        });
    }
    proposal.paired = Some(pair);
    Ok(Some((nets, proposal)))
}

pub(super) fn blocked_details(
    pcb: &Expr,
    codes: &BTreeMap<String, String>,
    opens: &[(&str, &[Value])],
    selected: &BTreeSet<String>,
    net: &str,
) -> Option<PairedSearch> {
    let (mate, _, positive) = project_rules::diff_pair_of(net)?;
    let nets = if positive { [net, &mate] } else { [&mate, net] };
    let mut pair = describe(pcb, codes, opens, nets);
    if pair.members.iter().any(|member| member.pads.is_empty()) {
        return None;
    }
    let (reason, next) = if pair
        .members
        .iter()
        .any(|member| member.retained_copper != 0)
    {
        (
            "pair_member_has_retained_copper",
            "Keep the existing copper. If both conductors are router-generated, compare a coupled reroute from the matching placed project using the original rules.",
        )
    } else if pair.members.iter().any(|member| !member.native_open) {
        (
            "pair_members_not_both_open",
            "Inspect the two members' native connectivity before making a coupled change.",
        )
    } else if !selected.contains(&mate) {
        (
            "pair_mate_not_selected",
            "Select both listed names with --net to try a coupled route.",
        )
    } else {
        (
            "pair_not_eligible_for_local_search",
            "Review the listed pad witnesses and the project's native reports before comparing a coupled route.",
        )
    };
    pair.reason = Some(reason);
    pair.next_action = Some(next);
    Some(pair)
}

fn describe(
    pcb: &Expr,
    codes: &BTreeMap<String, String>,
    opens: &[(&str, &[Value])],
    nets: [&str; 2],
) -> PairedSearch {
    let members = nets.map(|net| {
        let pads = pcb
            .children()
            .iter()
            .filter_map(|footprint| footprint_reference(footprint).map(|name| (footprint, name)))
            .flat_map(|(footprint, reference)| {
                footprint.children().iter().filter_map(move |pad| {
                    if pad.head() != Some("pad") || item_net(pad, codes) != Some(net) {
                        return None;
                    }
                    Some(PairedPad {
                        reference: reference.clone(),
                        pad: unescape(pad.children().get(1)?.atom()?).into_owned(),
                        uuid: item_id(pad),
                    })
                })
            })
            .collect();
        PairedMember {
            net: unescape(net).into_owned(),
            native_open: opens.iter().any(|(name, _)| *name == net),
            retained_copper: pcb
                .children()
                .iter()
                .filter(|item| item_net(item, codes) == Some(net))
                .count(),
            pads,
        }
    });
    PairedSearch {
        nets: nets.map(|net| unescape(net).into_owned()),
        members,
        reason: None,
        unmatched_end: None,
        next_action: None,
        geometry: None,
        stored_report: None,
    }
}

fn eligible(
    pcb: &Expr,
    codes: &BTreeMap<String, String>,
    opens: &[(&str, &[Value])],
    nets: [&str; 2],
) -> bool {
    if pcb
        .children()
        .iter()
        .any(|item| item_net(item, codes).is_some_and(|net| nets.contains(&net)))
    {
        return false;
    }
    nets.into_iter().all(|net| {
        let pads: Vec<_> = pcb
            .children()
            .iter()
            .filter(|item| matches!(item.head(), Some("footprint" | "module")))
            .flat_map(Expr::children)
            .filter(|item| item.head() == Some("pad") && item_net(item, codes) == Some(net))
            .map(item_id)
            .collect();
        let [Some(a), Some(b)] = pads.as_slice() else {
            return false;
        };
        let witnesses: Vec<_> = opens
            .iter()
            .filter_map(|&(name, items)| (name == net).then_some(items))
            .collect();
        let [[one, two]] = witnesses.as_slice() else {
            return false;
        };
        let id = |item: &Value| item["uuid"].as_str().map(str::to_ascii_lowercase);
        let ids = [id(one), id(two)];
        a != b && ids.contains(&Some(a.clone())) && ids.contains(&Some(b.clone()))
    })
}

fn route_pair(
    pcb: &Expr,
    mut rules: KiCadBoardRouterConfig,
    nets: [&str; 2],
    evidence: &mut PairedSearch,
) -> Result<Option<(Vec<Expr>, board_router::KiCadDiffPair)>, String> {
    rules.diff_pairs = Some(true);
    rules.diff_pair_return_vias = Some(false);
    rules.diff_pair_give_up = Some(false);
    rules.deadline = Some(Instant::now() + local::LOCAL_WALL);
    let names = NetNames::of(pcb);
    let keys = nets.map(|net| names.normalize(net));
    if keys[0] == keys[1] || rules.excludes_diff_pair(keys[0], keys[1]) {
        return Ok(None);
    }
    let (rules, mut board) = local::lower_for_local(pcb, board_router::with_pairs(&rules))?;
    let mut points = Vec::new();
    for net in &mut board.nets {
        if keys.contains(&net.name.as_str()) {
            if net.terminals.len() != 2 {
                return Ok(None);
            }
            points.extend(net.terminals.iter().map(|terminal| terminal.anchor));
        } else {
            net.terminals.clear();
        }
    }
    if points.len() != 4 {
        return Ok(None);
    }
    // Existing zone copper remains on the board and is checked in both
    // native modes. Estimated fills cannot be used as pair terminals.
    board.planes.clear();
    board.outline = outline_pieces::clip(&board.outline, local::window_of(&points).unwrap());
    let Some(size) = local::search_size(&board) else {
        evidence.reason = Some("coupled_local_search_limit");
        return Ok(None);
    };
    if board.outline.len() < 3 {
        return Ok(None);
    }
    let net_count = board.nets.len();
    board_router::plan_pairs(&mut board, &rules);
    if board.nets.len() != net_count + 1 {
        if let Some((reason, spacing)) = board_router::isolated_pair_plan_failure(&board) {
            evidence.reason = Some(reason);
            evidence.unmatched_end = spacing;
        }
        return Ok(None);
    }
    let maps_mb = size.maps_bytes_floor(board.layer_count, board.classes.len()) / 1_048_576.0;
    if board_router::MemoryLimits::read(None)
        .too_small_for(maps_mb, 1)
        .is_some()
    {
        evidence.reason = Some("coupled_local_search_limit");
        return Ok(None);
    }
    let mut config = board_router::core_config(&rules);
    config.pitches = vec![size.pitch];
    config.parallel = false;
    config.max_iterations = 2;
    config.cleanup_passes = 0;
    config.share_static_maps = false;
    config.lost_spoke_repair = false;
    config.trouble_repair = false;
    let mut routed = core::route(&board, &config);
    let layers = board_router::LayerTable::from_pcb(pcb)?.names;
    let heights = board_router::copper_heights(pcb);
    let heights: Vec<_> = layers
        .iter()
        .map(|name| heights.get(name).copied().unwrap_or(0.0))
        .collect();
    let Some((restored, copper, mut result)) =
        board_router::finish_pairs(&board, &mut routed, &rules, &heights)
    else {
        return Ok(None);
    };
    if result.coupled != 1 || result.links_coupled != 1 {
        return Ok(None);
    }
    let mut candidate = pcb.clone();
    board_router::write_pair_copper(&mut candidate, &copper, &layers)?;
    board_router::emit_routes(&mut candidate, &restored, &routed, &layers, true)?;
    board_router::measure_uncoupled(&mut result, &candidate);
    let codes = net_codes(pcb);
    let mut occupied = item_ids(pcb);
    let mut used = BTreeSet::new();
    let mut additions = Vec::new();
    for item in &candidate.children()[pcb.children().len()..] {
        if route_item(item) {
            let Some(name) = item_net(item, &codes) else {
                return Ok(None);
            };
            let Some(index) = keys.iter().position(|key| *key == names.normalize(name)) else {
                return Ok(None);
            };
            used.insert(index);
            let mut forms = item.children().to_vec();
            for form in forms.iter_mut().filter(|form| form.head() == Some("net")) {
                *form = source_net_field(&codes, nets[index]);
            }
            additions.push(Expr::List(forms));
        } else if item.head() == Some("generated") {
            additions.push(item.clone());
        } else {
            return Ok(None);
        }
        if item_id(item).is_none_or(|id| !occupied.insert(id)) {
            return Ok(None);
        }
    }
    Ok(result
        .pairs
        .into_iter()
        .next()
        .filter(|_| used.len() == 2)
        .map(|geometry| (additions, geometry)))
}

pub(super) fn findings_preserved(before: &Value, after: &Value) -> Result<bool, String> {
    Ok(!report_capped(before)
        && !report_capped(after)
        && finding_types_at_cap(before, "schematic_parity").is_empty()
        && finding_types_at_cap(after, "schematic_parity").is_empty()
        && preserved_findings(before, after)?)
}

pub(super) fn closes_both(before: &Value, after: &Value, pcb: &Expr, pair: &PairedSearch) -> bool {
    let was_open = open_nets(pcb, before);
    let still_open = open_nets(pcb, after);
    pair.nets.iter().all(|name| {
        was_open.iter().any(|raw| unescape(raw) == *name)
            && !still_open.iter().any(|raw| unescape(raw) == *name)
    }) && before["unconnected_items"]
        .as_array()
        .zip(after["unconnected_items"].as_array())
        .is_some_and(|(a, b)| a.len().checked_sub(b.len()) == Some(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const BOARD: &str = r#"(kicad_pcb
        (footprint "First" (property "Reference" "U1")
                           (pad "1" (net "USB_DP") (uuid "p1"))
                          (pad "2" (net "USB_DN") (uuid "n1")))
        (footprint "Second" (property "Reference" "U2")
                            (pad "1" (net "USB_DP") (uuid "p2"))
                           (pad "2" (net "USB_DN") (uuid "n2"))))"#;

    fn two_open_members() -> Value {
        json!({
            "violations": [],
            "unconnected_items": [
                {"items": [{"uuid": "p1"}, {"uuid": "p2"}]},
                {"items": [{"uuid": "n1"}, {"uuid": "n2"}]}
            ]
        })
    }

    #[test]
    fn paired_admission_requires_both_unstarted_two_pad_members() {
        let test = |board: &Expr, report: &Value| {
            let codes = net_codes(board);
            let opens = native_opens(board, report, &codes);
            eligible(board, &codes, &opens, ["USB_DP", "USB_DN"])
        };
        let open = two_open_members();
        let pcb = parse(BOARD).unwrap();
        assert!(test(&pcb, &open));
        let mut one = open.clone();
        one["unconnected_items"].as_array_mut().unwrap().pop();
        assert!(!test(&pcb, &one));
        let started = format!(
            "{} (segment (locked yes) (net \"USB_DN\") (uuid \"original\")))",
            BOARD.strip_suffix(')').unwrap()
        );
        assert!(!test(&parse(&started).unwrap(), &open));
    }

    #[test]
    fn pair_gate_requires_both_witnesses_and_every_other_finding_preserved() {
        let pcb = parse(BOARD).unwrap();
        let before = two_open_members();
        let codes = net_codes(&pcb);
        let pair = describe(
            &pcb,
            &codes,
            &native_opens(&pcb, &before, &codes),
            ["USB_DP", "USB_DN"],
        );
        let clean = json!({"violations": [], "unconnected_items": []});
        assert!(findings_preserved(&before, &clean).unwrap());
        assert!(closes_both(&before, &clean, &pcb, &pair));
        let mut one = before.clone();
        one["unconnected_items"].as_array_mut().unwrap().pop();
        assert!(!closes_both(&before, &one, &pcb, &pair));
        let mut skew = clean;
        skew["violations"] = json!([{"type":"skew_out_of_range", "severity":"error"}]);
        assert!(closes_both(&before, &skew, &pcb, &pair));
        assert!(!findings_preserved(&before, &skew).unwrap());
    }

    #[test]
    fn skipped_pair_names_the_kept_member_even_when_only_its_mate_was_selected() {
        let started = format!(
            "{} (segment (net \"USB_DN\") (uuid \"original\")))",
            BOARD.strip_suffix(')').unwrap()
        );
        let pcb = parse(&started).unwrap();
        let codes = net_codes(&pcb);
        let selected = BTreeSet::from(["USB_DP".to_string()]);
        let witnesses = json!([{"uuid": "p1"}, {"uuid": "p2"}]);
        let opens = [("USB_DP", witnesses.as_array().unwrap().as_slice())];
        let pair = blocked_details(&pcb, &codes, &opens, &selected, "USB_DP").unwrap();
        assert_eq!(pair.reason, Some("pair_member_has_retained_copper"));
        assert!(pair.members[0].native_open);
        assert_eq!(pair.members[1].retained_copper, 1);
        assert_eq!(pair.members[1].pads[1].reference, "U2");
        assert_eq!(pair.members[1].pads[1].uuid.as_deref(), Some("n2"));
    }
}

// Copyright (C) 2026 Florian Loitsch.

//! Bounded routing between the components of a native open net.

use super::*;
use outline_pieces::{NativeFindingLocation, NativePanelScope, Pieces};
use pcb_router as core;

const PITCHES_MM: &[f64] = &[0.025, 0.05, 0.075, 0.1, 0.127];
const WINDOW_MARGIN_MM: f64 = 11.0;
const MAX_LAYER_CELLS: f64 = 4_500_000.0;
const MAX_COMPONENTS: usize = 8;
const MAX_BRANCHES: usize = 4;
pub(super) const LOCAL_WALL: std::time::Duration = std::time::Duration::from_secs(20);
const POSITION_TOLERANCE_MM: f64 = 0.000_01;

#[derive(Serialize)]
pub struct LocalSearch {
    pub native_opens: usize,
    pub kept_components: usize,
    /// Terminal groups selected from KiCad's refilled copper partition.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refilled_components: Option<usize>,
    pub pitch_mm: f64,
    pub window_mm: [f64; 4],
    pub searches: u64,
    pub expanded_nodes: u64,
    pub routing_stop: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reserved_plane_layers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_terminals: Vec<board_router::KiCadDeadPad>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub completed: bool,
    /// A complementary name allowed to route independently under native rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_inferred_pair_partner: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attempted_branches: Vec<[core::Point; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub teardrop_contacts: Vec<teardrop::TeardropContact>,
    /// The explicitly listed through-via size that opened a local branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset_via_mm: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_footprint_pad_bridge: Option<pad_bridge::SameFootprintPadBridge>,
    #[serde(skip)]
    work_seconds: f64,
}

pub(super) struct Proposal {
    pub objects: Vec<Expr>,
    pub status: &'static str,
    pub search: Option<LocalSearch>,
    pub paired: Option<PairedSearch>,
}

struct Target<'a> {
    native_net: &'a str,
    net: &'a str,
    independent_partner: Option<String>,
    blocked: Option<&'static str>,
    named_single_net: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SearchScope {
    Ordinary,
    Named,
    LargeNet,
}

pub(super) fn large_open_nets(pcb: &Expr, native: &Value) -> BTreeSet<String> {
    let codes = net_codes(pcb);
    let mut counts = BTreeMap::<&str, usize>::new();
    for (net, _) in native_opens(pcb, native, &codes) {
        *counts.entry(net).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, branches)| *branches >= MAX_COMPONENTS)
        .map(|(net, _)| net.into())
        .collect()
}

impl Proposal {
    pub fn candidate(objects: Vec<Expr>) -> Self {
        Self {
            objects,
            status: "no_candidate_copper",
            search: None,
            paired: None,
        }
    }

    pub(super) fn skipped(status: &'static str) -> Self {
        Self {
            objects: Vec::new(),
            status,
            search: None,
            paired: None,
        }
    }
}

/// Keep other nets' obstacles, but route only nets identified by KiCad.
/// Removing the pours from the search avoids taking a modeled connection
/// through them as real. The native trial fills the original zones.
pub(super) fn propose(
    pcb: &Expr,
    path: &Path,
    native: &Value,
    nets: &BTreeSet<String>,
    pieces: Option<&Pieces>,
    search_scope: SearchScope,
    mut budget: Option<&mut LayoutRepairBudget<'_>>,
) -> Result<Vec<(String, Proposal)>, String> {
    if nets.is_empty() {
        return Ok(Vec::new());
    }
    if native["coordinate_units"] != "mm" {
        return Ok(nets
            .iter()
            .take(MAX_TRIALS)
            .map(|net| {
                (
                    net.clone(),
                    Proposal::skipped("unsupported_native_coordinates"),
                )
            })
            .collect());
    }
    let layers = board_router::LayerTable::from_pcb(pcb).map_err(exit::input_error)?;
    let codes = net_codes(pcb);
    let allow_teardrops = search_scope == SearchScope::Named
        && budget.is_none()
        && pieces.is_none()
        && !report_capped(native)
        && finding_types_at_cap(native, "schematic_parity").is_empty();
    let candidates = native_open_witnesses(pcb, native, &codes, allow_teardrops);
    let rules = if let Some(budget) = &budget {
        budget.router.clone()
    } else {
        project_rules::resolve_project_rules(&path.with_extension("kicad_pro"), path)?
    };
    let (rules, board) = lower_for_local(pcb, rules)?;
    let names = NetNames::of(pcb);
    let layout = budget.is_some();
    let pair_requirements = (layout && layers.len() == 2)
        .then(|| project_rules::LocalPairRequirements::read(path))
        .flatten();
    let pad_nets: BTreeSet<_> = pair_requirements
        .iter()
        .flat_map(|_| pcb.children())
        .filter(|item| matches!(item.head(), Some("footprint" | "module")))
        .flat_map(Expr::children)
        .filter(|pad| pad.head() == Some("pad"))
        .filter_map(|pad| item_net(pad, &codes))
        .collect();
    let has_length_rule = |name: &str| {
        rules
            .length_rules
            .iter()
            .any(|rule| rule.nets.iter().any(|member| member.net == name))
    };
    let mut targets: Vec<_> = nets
        .iter()
        .map(|net| {
            let name = names.normalize(net);
            let partner = project_rules::diff_pair_of(name)
                .map(|(partner, _, _)| partner)
                .filter(|partner| board.nets.iter().any(|other| other.name == *partner));
            let independent = partner.as_deref().and_then(|partner| {
                let native = pair_requirements.as_ref()?;
                let (actual_partner, _, _) = project_rules::diff_pair_of(net)?;
                if !pad_nets.contains(actual_partner.as_str())
                    || names.normalize(&actual_partner) != partner
                    || (rules.diff_pairs_on() && !rules.excludes_diff_pair(name, partner))
                    || [name, partner].into_iter().any(|net| {
                        has_length_rule(net)
                            || rules.diff_pair_rules.get(net).is_some_and(|rule| {
                                rule.asked.is_some()
                                    || rule.max_uncoupled.is_some()
                                    || rule.max_skew.is_some()
                                    || rule.length.is_some()
                            })
                    })
                    || native.protects(
                        [net, &actual_partner],
                        [name, partner],
                        &layers.names,
                        &rules,
                    )
                {
                    return None;
                }
                Some(actual_partner)
            });
            let blocked =
                if partner.is_some() && (layout || layers.len() > 2) && independent.is_none() {
                    Some("single_net_search_cannot_certify_pair_coupling")
                } else if layout && has_length_rule(name) {
                    Some("single_net_search_cannot_certify_length_rules")
                } else {
                    None
                };
            Target {
                native_net: net,
                net: name,
                independent_partner: independent,
                blocked,
                named_single_net: allow_teardrops && partner.is_none() && !has_length_rule(name),
            }
        })
        .collect();
    if let Some(budget) = &budget {
        if !budget.can_check_trial() {
            return Ok(Vec::new());
        }
        // A short native witness is a promising target in the one route
        // check available to automatic repair. Search can still decline
        // the candidate if clearance or scope makes it impractical.
        let mut nearest = BTreeMap::<&str, f64>::new();
        for &(name, items) in &candidates {
            let point =
                |item: &Value| Some([item["pos"]["x"].as_f64()?, item["pos"]["y"].as_f64()?]);
            if let [a, b] = items
                && let Some(([ax, ay], [bx, by])) = point(a).zip(point(b))
            {
                let distance = (ax - bx).hypot(ay - by);
                let best = nearest.entry(name).or_insert(distance);
                *best = best.min(distance);
            }
        }
        let span = |name: &str| nearest.get(name).copied().unwrap_or(f64::INFINITY);
        targets.sort_by(|a, b| {
            a.blocked
                .is_some()
                .cmp(&b.blocked.is_some())
                .then_with(|| span(a.native_net).total_cmp(&span(b.native_net)))
        });
    }
    let mut occupied = item_ids(pcb);
    let endpoint_layers = endpoint_layers(pcb, &layers);
    let mut panel = pieces.map(|pieces| NativePanelScope::new(pcb, pieces));
    let mut refilled = None;
    let filled_board = std::sync::OnceLock::new();
    let mut result = Vec::new();
    let bridge_checks = pad_bridge::checks_needed(pad_bridge::has_fills(pcb));
    for target in targets.into_iter().take(MAX_TRIALS) {
        if budget.as_ref().is_some_and(|budget| {
            if search_scope == SearchScope::LargeNet {
                budget.branch_search_limit() <= 0.0
            } else {
                budget.search_limit() <= 0.0
            }
        }) {
            break;
        }
        let mut teardrop_contacts = Vec::new();
        let pairs: Vec<_> = candidates
            .iter()
            .filter(|(name, _)| *name == target.native_net)
            .filter(|(_, items)| {
                panel.as_mut().is_none_or(|scope| {
                    matches!(scope.location(items), NativeFindingLocation::Piece(_))
                })
            })
            .filter_map(|(_, items)| {
                let [a, b] = *items else { return None };
                let resolve = |item: &Value| {
                    if let Some(endpoint) = endpoint(item, &endpoint_layers) {
                        return Some((endpoint, None));
                    }
                    if !target.named_single_net {
                        return None;
                    }
                    let witness = teardrop::witness(item, target.native_net)?;
                    let filled = filled_board.get_or_init(|| {
                        let deadline = Instant::now() + LOCAL_WALL;
                        native_report::read_refilled_board(path, || Instant::now() >= deadline).ok()
                    });
                    let contact = teardrop::contact(
                        pcb,
                        filled.as_ref()?,
                        &codes,
                        target.native_net,
                        witness,
                    )?;
                    let layers = *endpoint_layers.get(&contact.kept_item)?;
                    Some((
                        Endpoint {
                            at: contact.contact_mm,
                            layers,
                        },
                        Some(contact),
                    ))
                };
                let [(a_end, a_contact), (b_end, b_contact)] = [resolve(a)?, resolve(b)?];
                teardrop_contacts.extend([a_contact, b_contact].into_iter().flatten());
                Some(([a_end, b_end], [a["uuid"].as_str()?, b["uuid"].as_str()?]))
            })
            .collect();
        let mut proposal = if let Some(blocked) = target.blocked {
            let mut proposal = Proposal::skipped(blocked);
            if blocked == "single_net_search_cannot_certify_pair_coupling"
                && budget.is_none()
                && pieces.is_none()
            {
                proposal.paired =
                    paired::blocked_details(pcb, &codes, &candidates, nets, target.native_net);
            }
            proposal
        } else if pairs.is_empty() {
            Proposal::skipped("native_endpoints_unavailable")
        } else {
            let groups = if pieces.is_none()
                && search_scope != SearchScope::LargeNet
                && pairs.len() > 1
                && !report_capped(native)
                && board
                    .planes
                    .iter()
                    .any(|plane| board.nets[plane.net as usize].name == target.net)
            {
                if refilled.is_none() {
                    refilled = Some(refilled_partition(
                        path,
                        native,
                        rules.deadline,
                        budget.as_deref_mut(),
                    )?);
                }
                refilled
                    .as_ref()
                    .and_then(Option::as_ref)
                    .and_then(|partition| {
                        let native_net = unescape(target.native_net);
                        pairs
                            .iter()
                            .map(|(_, ids)| {
                                let [a, b] = ids.map(|id| partition.get(&id.to_ascii_lowercase()));
                                let ((a_net, a_group), (b_net, b_group)) = a.zip(b)?;
                                (a_net == native_net.as_ref()
                                    && b_net == native_net.as_ref()
                                    && a_group != b_group)
                                    .then_some([*a_group, *b_group])
                            })
                            .collect::<Option<Vec<_>>>()
                    })
            } else {
                None
            };
            let pairs: Vec<_> = pairs.into_iter().map(|(endpoints, _)| endpoints).collect();
            let scope = board
                .nets
                .iter()
                .enumerate()
                .filter(|(_, connection)| connection.name == target.net)
                .filter_map(|(index, _)| focus(&board, index, &pairs, groups.as_deref()))
                .max_by_key(|focus| (focus.native_opens, std::cmp::Reverse(focus.net)));
            scope.map_or_else(
                || Proposal::skipped("native_open_not_reproduced_in_local_model"),
                |scope| {
                    let net_form = source_net_field(&codes, target.native_net);
                    if (search_scope != SearchScope::Ordinary
                        && scope.components.len() > MAX_COMPONENTS)
                        || (!teardrop_contacts.is_empty() && pairs.len() > 1)
                    {
                        let mut branch_rules = rules.clone();
                        let end = Instant::now() + LOCAL_WALL;
                        let deadline = rules.deadline.map_or(end, |limit| limit.min(end));
                        branch_rules.deadline = Some(deadline);
                        let search_work =
                            budget.as_ref().map(|budget| budget.branch_search_limit());
                        return search_branches(
                            &board,
                            scope.net,
                            pairs,
                            |branch, remaining| {
                                search(
                                    &board,
                                    &branch_rules,
                                    &layers,
                                    target.net,
                                    net_form.clone(),
                                    branch,
                                    remaining,
                                )
                            },
                            deadline,
                            search_work,
                        );
                    }
                    if search_scope == SearchScope::LargeNet {
                        return Proposal::skipped("native_large_net_not_reproduced_in_local_model");
                    }
                    let started = Instant::now();
                    let net = scope.net;
                    let mut first = search(
                        &board,
                        &rules,
                        &layers,
                        target.net,
                        net_form.clone(),
                        scope,
                        budget.as_ref().map(|budget| budget.search_limit()),
                    );
                    if target.named_single_net
                        && layers.len() == 2
                        && first.objects.is_empty()
                        && let Some(size) =
                            compact_via(&rules, board.classes[board.nets[net].class])
                    {
                        let alternate = board_with_preset(board.clone(), net, size);
                        let deadline = started + LOCAL_WALL;
                        let mut branch_rules = rules.clone();
                        branch_rules.deadline = Some(deadline);
                        let mut alternative = search_branches(
                            &alternate,
                            net,
                            pairs,
                            |branch, remaining| {
                                search(
                                    &alternate,
                                    &branch_rules,
                                    &layers,
                                    target.net,
                                    net_form.clone(),
                                    branch,
                                    remaining,
                                )
                            },
                            deadline,
                            None,
                        );
                        if let Some(stats) = &mut alternative.search {
                            stats.preset_via_mm = Some(size);
                            if let Some(original) = first.search.take() {
                                stats.searches += original.searches;
                                stats.expanded_nodes += original.expanded_nodes;
                                stats.work_seconds += original.work_seconds;
                            }
                            return alternative;
                        }
                    }
                    first
                },
            )
        };
        if pieces.is_none()
            && search_scope != SearchScope::LargeNet
            && target.blocked.is_none()
            && project_rules::diff_pair_of(target.net)
                .is_none_or(|(partner, _, _)| !board.nets.iter().any(|net| net.name == partner))
            && proposal.objects.is_empty()
            && matches!(
                proposal.status,
                "no_local_path" | "component_without_legal_access"
            )
            && proposal.search.as_ref().is_some_and(|search| {
                budget.as_ref().is_none_or(|budget| {
                    budget.can_check_trials_after(bridge_checks, search.work_seconds)
                })
            })
        {
            let mut witnesses = candidates
                .iter()
                .filter(|(net, _)| *net == target.native_net);
            if let (Some((_, witness)), None) = (witnesses.next(), witnesses.next())
                && let Some((track, bridge)) = pad_bridge::propose(pad_bridge::Inputs {
                    pcb,
                    native,
                    board: &board,
                    rules: &rules,
                    layers: &layers,
                    native_net: target.native_net,
                    net: target.net,
                    witness,
                })
                && let Some(search) = &mut proposal.search
            {
                search.same_footprint_pad_bridge = Some(bridge);
                proposal.objects.push(track);
            }
        }
        if !proposal.objects.is_empty() {
            proposal
                .objects
                .retain(|item| item_id(item).is_some_and(|id| occupied.insert(id)));
        }
        if let (Some(budget), Some(search)) = (&mut budget, &proposal.search) {
            budget.search_work_seconds += search.work_seconds;
        }
        if let Some(search) = &mut proposal.search {
            search.name_inferred_pair_partner = target.independent_partner;
            search.teardrop_contacts = teardrop_contacts;
        }
        let ready = !proposal.objects.is_empty();
        result.push((target.native_net.to_owned(), proposal));
        if ready && budget.is_some() {
            break;
        }
    }
    Ok(result)
}

type NativePartition = BTreeMap<String, (String, usize)>;

fn board_with_preset(mut board: core::Board, net: usize, size: [f64; 2]) -> core::Board {
    let mut class = board.classes[board.nets[net].class];
    [class.via_diameter, class.via_drill] = size;
    board.nets[net].class = board.classes.len();
    board.classes.push(class);
    board
}

pub struct GeneratedBranch {
    pub copper: Vec<Expr>,
    pub via_mm: [f64; 2],
}

/// Measured geometry build and search work for every attempted branch.
#[derive(Default)]
pub struct GeneratedBranchWork {
    pub limit_seconds: Option<f64>,
    pub work_seconds: f64,
    pub searches: u64,
    pub expanded_nodes: u64,
}

impl GeneratedBranchWork {
    pub fn exhausted(&self) -> bool {
        self.remaining().is_some_and(|seconds| seconds <= 0.0)
    }

    fn remaining(&self) -> Option<f64> {
        self.limit_seconds.map(|limit| limit - self.work_seconds)
    }
}

/// Search a single native open while retaining every piece of copper on
/// the supplied board, including the trial position of a generated via.
#[allow(clippy::too_many_arguments)]
pub fn propose_generated_branch(
    pcb: &Expr,
    path: &Path,
    native: &Value,
    native_net: &str,
    moved_net: &str,
    branch_ids: [&str; 2],
    deadline: Instant,
    compact: bool,
    work: &mut GeneratedBranchWork,
) -> Result<Option<GeneratedBranch>, String> {
    if Instant::now() >= deadline
        || work.exhausted()
        || native["coordinate_units"] != "mm"
        || report_capped(native)
        || !finding_types_at_cap(native, "schematic_parity").is_empty()
    {
        return Ok(None);
    }
    let layers = board_router::LayerTable::from_pcb(pcb).map_err(exit::input_error)?;
    if (compact && layers.len() != 2) || Pieces::of_board(pcb)?.is_some() {
        return Ok(None);
    }
    let codes = net_codes(pcb);
    let masks = endpoint_layers(pcb, &layers);
    let pair = native_opens(pcb, native, &codes)
        .into_iter()
        .find_map(|(net, items)| {
            let [a, b] = items else { return None };
            let ids = [a["uuid"].as_str()?, b["uuid"].as_str()?];
            (net == native_net && (ids == branch_ids || [ids[1], ids[0]] == branch_ids))
                .then(|| Some([endpoint(a, &masks)?, endpoint(b, &masks)?]))
                .flatten()
        });
    let Some(pair) = pair else { return Ok(None) };
    let names = NetNames::of(pcb);
    let name = names.normalize(native_net);
    let moved = names.normalize(moved_net);
    let mut rules = project_rules::resolve_project_rules(&path.with_extension("kicad_pro"), path)?;
    if rules.length_rules.iter().any(|rule| {
        rule.nets
            .iter()
            .any(|member| member.net == name || member.net == moved)
    }) {
        return Ok(None);
    }
    rules.deadline = Some(deadline);
    let (rules, board) = lower_for_local(pcb, rules)?;
    if [name, moved].into_iter().any(|name| {
        project_rules::diff_pair_of(name)
            .is_some_and(|(partner, _, _)| board.nets.iter().any(|net| net.name == partner))
    }) || board
        .planes
        .iter()
        .any(|plane| board.nets[plane.net as usize].name == moved)
    {
        return Ok(None);
    }
    let Some(scope) = board
        .nets
        .iter()
        .enumerate()
        .filter(|(_, net)| net.name == name)
        .find_map(|(index, _)| focus(&board, index, &[pair], None))
    else {
        return Ok(None);
    };
    let net = scope.net;
    let class = board.classes[board.nets[net].class];
    let size = if compact {
        let Some(size) = compact_via(&rules, class) else {
            return Ok(None);
        };
        Some(size)
    } else {
        None
    };
    let board = if let Some(size) = size {
        board_with_preset(board, net, size)
    } else {
        board
    };
    let fields = source_net_field(&codes, native_net);
    let proposal = search(
        &board,
        &rules,
        &layers,
        name,
        fields,
        scope,
        work.remaining(),
    );
    if let Some(stats) = &proposal.search {
        work.work_seconds += stats.work_seconds;
        work.searches += stats.searches;
        work.expanded_nodes += stats.expanded_nodes;
    }
    let found = if compact {
        proposal
            .objects
            .iter()
            .any(|item| item.head() == Some("via"))
    } else {
        !proposal.objects.is_empty()
    };
    Ok((found && !work.exhausted()).then_some(GeneratedBranch {
        copper: proposal.objects,
        via_mm: size.unwrap_or([class.via_diameter, class.via_drill]),
    }))
}

fn compact_via(rules: &KiCadBoardRouterConfig, class: core::board::RuleClass) -> Option<[f64; 2]> {
    let report = rules.project_rules.as_ref()?;
    if report.custom_rules_error.is_some()
        || report.custom_rules.iter().any(|rule| {
            matches!(
                rule.constraint.as_str(),
                "via_diameter" | "hole_size" | "annular_width"
            )
        })
    {
        return None;
    }
    let [diameter_min, drill_min, annulus_min] = report.via_minimums;
    report
        .via_sizes
        .iter()
        .copied()
        .filter(|&[diameter, drill]| {
            diameter.is_finite()
                && drill.is_finite()
                && diameter + 1e-9 >= diameter_min
                && drill + 1e-9 >= drill_min
                && (diameter - drill) / 2.0 + 1e-9 >= annulus_min
                && drill > 0.0
                && diameter > drill
                && diameter < class.via_diameter
                && drill <= class.via_drill
        })
        .max_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])))
}

fn refilled_partition(
    path: &Path,
    report: &Value,
    deadline: Option<Instant>,
    budget: Option<&mut LayoutRepairBudget<'_>>,
) -> Result<Option<NativePartition>, String> {
    let Some(version) = report["kicad_version"].as_str() else {
        return Ok(None);
    };
    if let Some(budget) = budget {
        if !budget.can_check_trials(2) || budget.search_limit() <= budget.check_seconds {
            return Ok(None);
        }
        budget.native_checks += 1;
    }
    let original = fs::read(path).map_err(|error| error.to_string())?;
    let limit = Instant::now() + LOCAL_WALL;
    let end = deadline.map_or(limit, |deadline| deadline.min(limit));
    let result =
        native_report::connectivity_partition(path, version, true, || Instant::now() >= end);
    if fs::read(path).map_err(|error| error.to_string())? != original {
        return Err("KiCad's connectivity check modified the board".into());
    }
    Ok(result.ok().map(|groups| {
        groups
            .into_iter()
            .enumerate()
            .flat_map(|(group, (net, items))| {
                items.into_iter().map(move |id| (id, (net.clone(), group)))
            })
            .collect()
    }))
}

pub fn lower_for_local(
    pcb: &Expr,
    mut rules: KiCadBoardRouterConfig,
) -> Result<(KiCadBoardRouterConfig, core::Board), String> {
    rules.bga_fanout = Some(false);
    rules.via_in_pad = Some(KiCadViaInPad::All(false));
    let mut board = board_router::lower(pcb, &rules, false)?.board;
    board_router::keep_existing_copper(&mut board, true);
    Ok((rules, board))
}

/// Route just two physical pads, keeping the original copper as obstacles
/// without collapsing the two pads into an already-connected terminal.
pub fn propose_direct_pad_path(
    pcb: &Expr,
    path: &Path,
    native_net: &str,
    pads: [(&str, core::Point, core::LayerMask); 2],
) -> Result<(Vec<Expr>, Option<LocalSearch>), String> {
    let mut rules = project_rules::resolve_project_rules(&path.with_extension("kicad_pro"), path)?;
    rules.bga_fanout = Some(false);
    rules.via_in_pad = Some(KiCadViaInPad::All(false));
    rules.tuning.sub_board_nets = Some(false);
    let board = board_router::lower(pcb, &rules, false)?.board;
    let layers = board_router::LayerTable::from_pcb(pcb)?;
    let names = NetNames::of(pcb);
    let name = names.normalize(native_net);
    let Some((net, description)) = board
        .nets
        .iter()
        .enumerate()
        .find(|(_, net)| net.name == name)
    else {
        return Ok((Vec::new(), None));
    };
    let terminals = pads.map(|(label, center, layers)| {
        let mut found = description
            .terminals
            .iter()
            .enumerate()
            .filter(|(_, terminal)| {
                terminal.label == label && terminal.anchor == center && terminal.layers == layers
            });
        let (index, _) = found.next()?;
        found.next().is_none().then_some(index)
    });
    let [Some(first), Some(second)] = terminals else {
        return Ok((Vec::new(), None));
    };
    if first == second {
        return Ok((Vec::new(), None));
    }
    let positions = pads.map(|(_, at, _)| at);
    let scope = Focus {
        net,
        components: BTreeSet::from([first, second]),
        window: window_of(&positions).expect("two pad positions"),
        native_opens: 0,
        refilled_components: None,
    };
    let codes = net_codes(pcb);
    let proposal = search(
        &board,
        &rules,
        &layers,
        name,
        source_net_field(&codes, native_net),
        scope,
        Some(8.0),
    );
    Ok((proposal.objects, proposal.search))
}

/// The standalone command reads the project settings afresh. Compare its
/// actual lowered inputs, including layout-only rule and geometry changes.
pub fn same_as_standalone(
    searched: &Path,
    selected: &Path,
    native: [&Value; 2],
    config: &KiCadBoardRouterConfig,
    available: f64,
) -> (bool, f64) {
    let compare = || -> Result<(bool, f64), String> {
        let (_, searched) = read_board(searched)?;
        let (_, published) = read_board(selected)?;
        if !same_native_search_inputs(&searched, &published, native) {
            return Ok((false, 0.0));
        }
        let work = board_router::lowering_work(&searched) + board_router::lowering_work(&published);
        if work > available {
            return Ok((false, 0.0));
        }
        let compare_models = || -> Result<bool, String> {
            let standalone = project_rules::resolve_project_rules(
                &selected.with_extension("kicad_pro"),
                selected,
            )?;
            let (layout, searched) = lower_for_local(&searched, config.clone())?;
            let (standalone, published) = lower_for_local(&published, standalone)?;
            Ok(layout.net_rules == standalone.net_rules && searched == published)
        };
        Ok((compare_models().unwrap_or(false), work))
    };
    compare().unwrap_or((false, 0.0))
}

fn endpoint_layers(
    pcb: &Expr,
    layers: &board_router::LayerTable,
) -> BTreeMap<String, core::LayerMask> {
    pcb.children()
        .iter()
        .flat_map(|item| std::iter::once(item).chain(item.children()))
        .filter_map(|item| {
            let mask = match item.head()? {
                "pad" => layers.pad_mask(item).ok()?,
                "segment" | "arc" => layers.mask_of_item(item).ok()?,
                "via" => layers.via_mask(item),
                _ => return None,
            };
            Some((item_id(item)?, mask))
        })
        .collect()
}

fn same_native_search_inputs(before: &Expr, after: &Expr, [a, b]: [&Value; 2]) -> bool {
    if a["coordinate_units"] != "mm"
        || b["coordinate_units"] != "mm"
        || !same_check_policy(a, b)
        || report_capped(a)
        || report_capped(b)
        || FINDING_KEYS
            .into_iter()
            .filter(|key| *key != "unconnected_items")
            .any(|key| {
                finding_counts(a, key)
                    .ok()
                    .zip(finding_counts(b, key).ok())
                    .is_none_or(|(a, b)| a != b)
            })
    {
        return false;
    }
    if a["unconnected_items"] == b["unconnected_items"] {
        return true;
    }
    // On a single board, KiCad may interchange a pad and a track at the
    // same connection point. Local repair uses the net, position, and
    // layer mask to find its kept-copper component.
    let opens = |pcb: &Expr, native: &Value| {
        if !Pieces::of_board(pcb).ok()?.is_none() {
            return None;
        }
        let layers = board_router::LayerTable::from_pcb(pcb).ok()?;
        let layers = endpoint_layers(pcb, &layers);
        let codes = net_codes(pcb);
        let opens = native_opens(pcb, native, &codes);
        if opens.len() != native["unconnected_items"].as_array()?.len() {
            return None;
        }
        opens
            .into_iter()
            .map(|(net, items)| {
                let [a, b] = items else { return None };
                Some((
                    net.to_owned(),
                    [endpoint(a, &layers)?, endpoint(b, &layers)?],
                ))
            })
            .collect::<Option<Vec<_>>>()
    };
    let Some((a, mut b)) = opens(before, a).zip(opens(after, b)) else {
        return false;
    };
    a.len() == b.len()
        && a.into_iter().all(|(net, [a1, a2])| {
            let Some(index) = b.iter().position(|(other, [b1, b2])| {
                *other == net && (a1 == *b1 && a2 == *b2 || a1 == *b2 && a2 == *b1)
            }) else {
                return false;
            };
            b.swap_remove(index);
            true
        })
}

#[derive(Clone, Copy, PartialEq)]
struct Endpoint {
    at: core::Point,
    layers: core::LayerMask,
}

fn endpoint(item: &Value, layers: &BTreeMap<String, core::LayerMask>) -> Option<Endpoint> {
    Some(Endpoint {
        at: [item["pos"]["x"].as_f64()?, item["pos"]["y"].as_f64()?],
        layers: *layers.get(&item["uuid"].as_str()?.to_ascii_lowercase())?,
    })
}

struct Focus {
    net: usize,
    components: BTreeSet<usize>,
    window: [f64; 4],
    native_opens: usize,
    refilled_components: Option<usize>,
}

/// A native endpoint's location on specific copper layers ties it to one
/// modeled component. Skip ambiguity, including different modeled roots
/// touching one endpoint.
fn focus(
    board: &core::Board,
    net: usize,
    pairs: &[[Endpoint; 2]],
    groups: Option<&[[usize; 2]]>,
) -> Option<Focus> {
    let connection = &board.nets[net];
    let component = |endpoint: Endpoint| {
        let touches = |shape: &core::Shape, layers: core::LayerMask| {
            layers & endpoint.layers != 0
                && shape.distance_to_point(endpoint.at) <= POSITION_TOLERANCE_MM
        };
        let mut matches = BTreeSet::new();
        for (index, terminal) in connection.terminals.iter().enumerate() {
            let pad = &board.obstacles[terminal.pad];
            if touches(terminal.contact.as_ref().unwrap_or(&pad.shape), pad.layers) {
                matches.insert(index);
            }
        }
        for (index, obstacle) in board
            .merged_pads
            .iter()
            .filter(|pad| pad.net as usize == net)
            .map(|pad| (pad.host, pad.pad))
            .chain(
                board
                    .terminal_copper
                    .iter()
                    .filter(|extra| extra.net as usize == net)
                    .map(|extra| (extra.terminal, extra.obstacle)),
            )
        {
            let obstacle = &board.obstacles[obstacle];
            if touches(&obstacle.shape, obstacle.layers) {
                matches.insert(index);
            }
        }
        (matches.len() == 1).then(|| *matches.first().unwrap())
    };
    let mut components = BTreeSet::new();
    let mut positions = Vec::new();
    let mut witnesses = Vec::new();
    for (index, &[a, b]) in pairs.iter().enumerate() {
        if let (Some(first), Some(second)) = (component(a), component(b))
            && first != second
        {
            components.extend([first, second]);
            positions.extend([a.at, b.at]);
            if let Some([a_group, b_group]) = groups.and_then(|groups| groups.get(index)).copied() {
                let span = (a.at[0] - b.at[0]).hypot(a.at[1] - b.at[1]);
                witnesses.extend([(first, a_group, span), (second, b_group, span)]);
            }
        }
    }
    let mut refilled_components = None;
    if witnesses.len() == 2 * pairs.len()
        && let Some(chosen) = choose_refilled_components(&witnesses)
        && (2..components.len()).contains(&chosen.len())
    {
        refilled_components = Some(chosen.len());
        components = chosen;
    }
    Some(Focus {
        net,
        components,
        window: window_of(&positions)?,
        native_opens: positions.len() / 2,
        refilled_components,
    })
}

// Use one routable contact per refilled group. The shortest witness keeps
// its nearest contact to another group, while the repair can still reach
// any connected saved copper of that contact's modeled terminal.
fn choose_refilled_components(witnesses: &[(usize, usize, f64)]) -> Option<BTreeSet<usize>> {
    let mut group_of = BTreeMap::new();
    let mut contacts = BTreeMap::<usize, (f64, usize)>::new();
    for &(terminal, group, span) in witnesses {
        if group_of
            .insert(terminal, group)
            .is_some_and(|previous| previous != group)
        {
            return None;
        }
        let nearest = contacts.entry(group).or_insert((span, terminal));
        if span
            .total_cmp(&nearest.0)
            .then(terminal.cmp(&nearest.1))
            .is_lt()
        {
            *nearest = (span, terminal);
        }
    }
    Some(contacts.values().map(|&(_, terminal)| terminal).collect())
}

pub(super) fn window_of(positions: &[[f64; 2]]) -> Option<[f64; 4]> {
    let [x, y] = *positions.first()?;
    let [left, top, right, bottom] = positions.iter().fold([x, y, x, y], |r, &[x, y]| {
        [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)]
    });
    Some([
        left - WINDOW_MARGIN_MM,
        top - WINDOW_MARGIN_MM,
        right + WINDOW_MARGIN_MM,
        bottom + WINDOW_MARGIN_MM,
    ])
}

fn search_branches(
    board: &core::Board,
    net: usize,
    mut pairs: Vec<[Endpoint; 2]>,
    mut search: impl FnMut(Focus, Option<f64>) -> Proposal,
    deadline: Instant,
    work_limit: Option<f64>,
) -> Proposal {
    let length = |pair: &[Endpoint; 2]| {
        let [[ax, ay], [bx, by]] = pair.map(|end| end.at);
        (ax - bx).hypot(ay - by)
    };
    pairs.sort_by(|a, b| length(a).total_cmp(&length(b)));
    let mut branches = Vec::new();
    let mut searches = 0;
    let mut expanded = 0;
    let mut work = 0.0;
    let mut result = Proposal::skipped("no_local_branch_path");
    for (pair, scope) in pairs
        .iter()
        .filter_map(|pair| {
            focus(board, net, std::slice::from_ref(pair), None).map(|scope| (pair, scope))
        })
        .take(MAX_BRANCHES)
    {
        let remaining = work_limit.map(|limit| limit - work);
        if Instant::now() >= deadline || remaining.is_some_and(|left| left <= 0.0) {
            break;
        }
        let mut proposal = search(scope, remaining);
        if let Some(stats) = &mut proposal.search {
            branches.push(pair.map(|end| end.at));
            searches += stats.searches;
            expanded += stats.expanded_nodes;
            work += stats.work_seconds;
            stats.searches = searches;
            stats.expanded_nodes = expanded;
            stats.work_seconds = work;
            stats.completed = false;
            stats.attempted_branches.clone_from(&branches);
        }
        result = proposal;
        if !result.objects.is_empty() {
            break;
        }
        result.status = "no_local_branch_path";
    }
    result
}

/// Preserve fine-pitch repair for small windows and choose the finest
/// lattice that fits the same allocation limit for more distant opens.
pub(super) fn search_size(board: &core::Board) -> Option<core::grid::LatticeSize> {
    PITCHES_MM
        .iter()
        .map(|&pitch| core::grid::LatticeSize::of(board, pitch))
        .find(|size| {
            size.check().is_ok() && size.cells() * board.layer_count as f64 <= MAX_LAYER_CELLS
        })
}

fn search(
    board: &core::Board,
    rules: &KiCadBoardRouterConfig,
    layers: &board_router::LayerTable,
    name: &str,
    net_form: Expr,
    scope: Focus,
    deadline_work: Option<f64>,
) -> Proposal {
    let Focus {
        net,
        components,
        window,
        native_opens,
        refilled_components,
    } = scope;
    let mut proposal = Proposal {
        search: Some(LocalSearch {
            native_opens,
            kept_components: components.len(),
            refilled_components,
            pitch_mm: PITCHES_MM[0],
            window_mm: window,
            searches: 0,
            expanded_nodes: 0,
            routing_stop: None,
            reserved_plane_layers: Vec::new(),
            blocked_terminals: Vec::new(),
            completed: false,
            name_inferred_pair_partner: None,
            attempted_branches: Vec::new(),
            teardrop_contacts: Vec::new(),
            preset_via_mm: None,
            same_footprint_pad_bridge: None,
            work_seconds: 0.0,
        }),
        ..Proposal::skipped("no_local_path")
    };
    if components.len() > MAX_COMPONENTS {
        proposal.status = "local_component_limit";
        return proposal;
    }
    let mut local = board.clone();
    if local.layer_count > 2 {
        let reserved = local
            .planes
            .iter()
            .filter(|plane| plane.exclusive && plane.net as usize != net)
            .fold(0, |mask, plane| mask | 1 << plane.layer);
        let class = &mut local.classes[local.nets[net].class];
        let original = if class.layers == 0 {
            layers.all()
        } else {
            class.layers
        };
        if let Some(search) = &mut proposal.search {
            search.reserved_plane_layers = layers
                .names
                .iter()
                .enumerate()
                .filter(|(layer, _)| reserved & original & (1 << layer) != 0)
                .map(|(_, name)| name.clone())
                .collect();
        }
        if original & !reserved == 0 {
            proposal.status = "all_route_layers_reserved_for_foreign_planes";
            return proposal;
        }
        class.layers = original & !reserved;
    }
    // Estimated fills never count as existing electrical contacts. The
    // trial uses the board's original zones and KiCad's native refill.
    local.planes.clear();
    local.outline = outline_pieces::clip(&local.outline, window);
    if local.outline.len() < 3 {
        proposal.status = "local_window_outside_board";
        return proposal;
    }
    let Some(size) = search_size(&local) else {
        proposal.status = "local_window_limit";
        return proposal;
    };
    if let Some(search) = &mut proposal.search {
        search.pitch_mm = size.pitch;
    }
    let maps_mb = size.maps_bytes_floor(local.layer_count, local.classes.len()) / 1_048_576.0;
    if board_router::MemoryLimits::read(None)
        .too_small_for(maps_mb, 1)
        .is_some()
    {
        proposal.status = "local_memory_limit";
        return proposal;
    }
    for connection in &mut local.nets {
        connection.terminals.clear();
    }
    let mut renumbered = BTreeMap::new();
    for old in components {
        renumbered.insert(old, local.nets[net].terminals.len());
        local.nets[net]
            .terminals
            .push(board.nets[net].terminals[old].clone());
    }
    local.merged_pads.retain_mut(|pad| {
        if pad.net as usize == net
            && let Some(index) = renumbered.get(&pad.host)
        {
            pad.host = *index;
            true
        } else {
            false
        }
    });
    local.terminal_copper.retain_mut(|extra| {
        if extra.net as usize == net
            && let Some(index) = renumbered.get(&extra.terminal)
        {
            extra.terminal = *index;
            true
        } else {
            false
        }
    });
    let local_deadline = Instant::now() + LOCAL_WALL;
    let config = core::router::Config {
        pitches: vec![size.pitch],
        via_cost: 2.0,
        bend_cost: 0.05,
        against_direction: 1.0,
        neck_reach: 0.0,
        pin_row_cost: 0.0,
        max_iterations: 1,
        window_margin: WINDOW_MARGIN_MM,
        cleanup_passes: 0,
        bend_round: false,
        via_reduction_rounds: 0,
        lost_spoke_repair: false,
        trouble_repair: false,
        parallel: false,
        pin_lines: core::pin_lines::Lines::Off,
        share_static_maps: false,
        deadline: Some(
            rules
                .deadline
                .map_or(local_deadline, |limit| limit.min(local_deadline)),
        ),
        deadline_work,
        limits_count_all: deadline_work.is_some(),
        net_rules: rules.net_rules.clone(),
        ..core::router::Config::default()
    };
    let routed = core::router::route(&local, &config);
    if let Some(search) = &mut proposal.search {
        search.searches = routed.searches;
        search.expanded_nodes = routed.expansions;
        search.routing_stop = Some(routed.negotiation_stop);
        search.work_seconds = routed.build_work
            + routed.pour_work
            + routed.expansions as f64 / core::router::expansions_per_second();
        search.completed = routed.negotiation_stop == "conflict_free"
            && routed.searches > 0
            && config.deadline.is_some_and(|end| Instant::now() < end)
            && deadline_work.is_none_or(|limit| search.work_seconds < limit)
            && routed
                .diagnostics
                .repair
                .iter()
                .any(|entry| entry.net as usize == net && entry.skipped.is_empty());
        search.blocked_terminals = routed
            .diagnostics
            .dead_pads
            .iter()
            .map(|dead| board_router::KiCadDeadPad::of(dead, &local, &layers.names))
            .collect();
    }
    let route = &routed.routes[net];
    let layers = &layers.names;
    let round = |point: core::Point| point.map(|v| (v * 1e6).round() / 1e6);
    let segments = route.segments.iter().map(|segment| {
        supplemental_segment_expr(&SupplementalSegment {
            connection: name.to_owned(),
            start: round(segment.start),
            end: round(segment.end),
            width: segment.width,
            layer: layers[segment.layer].clone(),
        })
    });
    let vias = route.vias.iter().map(|via| {
        let (layers, kind) = board_router::via_layers_and_kind(via.span, local.layer_count, layers);
        supplemental_via_expr(&SupplementalVia {
            connection: name.to_owned(),
            at: round(via.at),
            size: via.diameter,
            drill: via.drill,
            layers,
            kind,
        })
    });
    proposal.objects = segments
        .chain(vias)
        .map(|mut item| {
            if let Expr::List(parts) = &mut item {
                for part in parts.iter_mut().filter(|part| part.head() == Some("net")) {
                    *part = net_form.clone();
                }
            }
            item
        })
        .collect();
    if proposal.objects.is_empty()
        && proposal
            .search
            .as_ref()
            .is_some_and(|search| !search.blocked_terminals.is_empty())
    {
        proposal.status = "component_without_legal_access";
    }
    proposal
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_kicad_file::connection_rules::KiCadConnectionRoutingRules;

    #[test]
    fn the_compact_via_must_be_listed_and_keep_board_minimums() {
        let mut config = KiCadBoardRouterConfig {
            project_rules: Some(project_rules::KiCadProjectRulesReport {
                via_sizes: vec![[0.45, 0.25], [0.5, 0.3], [0.55, 0.4], [0.6, 0.4]],
                via_minimums: [0.5, 0.3, 0.1],
                ..Default::default()
            }),
            ..Default::default()
        };
        let class = core::board::RuleClass {
            trace_width: 0.127,
            clearance: 0.127,
            via_diameter: 0.6,
            via_drill: 0.4,
            layers: 0,
        };
        assert_eq!(compact_via(&config, class), Some([0.5, 0.3]));
        config.project_rules.as_mut().unwrap().custom_rules_error = Some("invalid rules".into());
        assert_eq!(compact_via(&config, class), None);
    }

    #[test]
    fn refilled_groups_keep_one_contact_at_the_shortest_native_witness() {
        let witnesses = [(5, 0, 2.0), (9, 1, 2.0), (7, 0, 0.3), (8, 2, 0.3)];
        assert_eq!(
            choose_refilled_components(&witnesses),
            Some(BTreeSet::from([7, 8, 9]))
        );
        let mut inconsistent = witnesses.to_vec();
        inconsistent.push((7, 2, 0.3));
        assert!(choose_refilled_components(&inconsistent).is_none());
    }

    #[test]
    fn the_retry_equivalence_detects_layout_only_rules_and_budget_limits() {
        let board =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plated-branch.kicad_pcb");
        let mut config =
            project_rules::resolve_project_rules(&board.with_extension("kicad_pro"), &board)
                .unwrap();
        let report =
            serde_json::json!({"coordinate_units":"mm", "violations":[], "unconnected_items":[]});
        let native = [&report, &report];
        let (same, work) = same_as_standalone(&board, &board, native, &config, 1.0);
        assert!(same && work > 0.0);
        assert_eq!(
            same_as_standalone(&board, &board, native, &config, 0.0),
            (false, 0.0)
        );
        config.edge_clearance_mm += 0.2;
        assert!(!same_as_standalone(&board, &board, native, &config, 1.0).0);
    }

    #[test]
    fn a_pad_and_track_witness_are_equivalent_at_the_same_endpoint() {
        let board = parse(r#"(kicad_pcb
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
            (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts") (stroke (width 0.05) (type default)))
            (footprint "Left" (layer "F.Cu") (at 2 10)
              (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "LINK") (uuid "left-pad")))
            (footprint "Right" (layer "F.Cu") (at 18 10)
              (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "LINK") (uuid "right-pad")))
            (segment (start 2 10) (end 7 10) (width 0.25) (layer "F.Cu") (net "LINK") (uuid "front-track"))
            (segment (start 2 10) (end 7 10) (width 0.25) (layer "B.Cu") (net "LINK") (uuid "back-track")))"#).unwrap();
        let report = |left: &str| {
            serde_json::json!({
                "coordinate_units": "mm",
                "violations": [],
                "unconnected_items": [{"items": [
                    {"uuid": left, "pos": {"x": 2.0, "y": 10.0}},
                    {"uuid": "right-pad", "pos": {"x": 18.0, "y": 10.0}}
                ]}]
            })
        };
        let pad = report("left-pad");
        let front = report("front-track");
        assert!(same_native_search_inputs(&board, &board, [&pad, &front]));
        let back = report("back-track");
        assert!(!same_native_search_inputs(&board, &board, [&pad, &back]));
    }

    #[test]
    fn existing_track_and_plated_via_belong_to_the_same_local_component() {
        let mut pcb = parse(include_str!("../../tests/fixtures/plated-branch.kicad_pcb")).unwrap();
        let Expr::List(items) = &mut pcb else {
            panic!()
        };
        items.pop(); // Disconnect J3 from the two pads and via at the left.
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.7,
                via_drill_mm: 0.35,
            }),
            ..Default::default()
        };
        let mut board = board_router::lower(&pcb, &config, false).unwrap().board;
        board_router::keep_existing_copper(&mut board, true);
        let track = Endpoint {
            at: [6.0, 10.0],
            layers: 1,
        };
        let via = Endpoint {
            at: [12.0, 10.0],
            layers: 2,
        };
        let pad = Endpoint {
            at: [18.0, 10.0],
            layers: 1,
        };
        assert!(focus(&board, 0, &[[track, via]], None).is_none());
        assert!(focus(&board, 0, &[[Endpoint { layers: 2, ..track }, pad]], None).is_none());
        let measured = focus(&board, 0, &[[track, pad]], None).unwrap();
        assert_eq!(measured.components.len(), 2);
        assert_eq!(measured.native_opens, 1);
    }

    #[test]
    fn distant_opens_get_the_finest_pitch_within_the_local_cell_cap() {
        let pcb = parse(include_str!("../../tests/fixtures/plated-branch.kicad_pcb")).unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.7,
                via_drill_mm: 0.35,
            }),
            ..Default::default()
        };
        let mut board = board_router::lower(&pcb, &config, false).unwrap().board;
        assert_eq!(search_size(&board).unwrap().pitch, 0.025);

        board.outline = vec![[0.0, 0.0], [158.0, 0.0], [158.0, 105.0], [0.0, 105.0]];
        let size = search_size(&board).unwrap();
        assert_eq!(size.pitch, 0.1);
        assert!(size.cells() * board.layer_count as f64 <= MAX_LAYER_CELLS);

        board.outline = vec![[0.0, 0.0], [500.0, 0.0], [500.0, 500.0], [0.0, 500.0]];
        assert!(search_size(&board).is_none());
    }

    #[test]
    fn a_local_repair_does_not_route_tracks_on_a_foreign_dedicated_plane() {
        let pcb = parse(
            r#"(kicad_pcb (version 20241229)
              (layers (0 "F.Cu" signal) (4 "In1.Cu" power)
                (6 "In2.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
              (gr_rect (start 0 0) (end 20 20) (layer "Edge.Cuts"))
              (footprint "test" (layer "F.Cu") (at 2 10)
                (property "Reference" "J1")
                (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4)
                   (layers "*.Cu") (net "/OPEN")))
              (footprint "test" (layer "F.Cu") (at 18 10)
                (property "Reference" "J2")
                (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4)
                   (layers "*.Cu") (net "/OPEN"))
                (pad "2" thru_hole circle (at 0 2) (size 1 1) (drill 0.4)
                   (layers "*.Cu") (net "GND")))
              (zone (net "GND") (layer "In1.Cu")
                (connect_pads (clearance 0.25)) (min_thickness 0.25)
                (polygon (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 0 20)))))"#,
        )
        .unwrap();
        let config = KiCadBoardRouterConfig {
            default_rules: Some(KiCadConnectionRoutingRules {
                trace_width_mm: 0.25,
                clearance_mm: 0.2,
                via_size_mm: 0.7,
                via_drill_mm: 0.35,
            }),
            ..Default::default()
        };
        let layers = board_router::LayerTable::from_pcb(&pcb).unwrap();
        let mut board = board_router::lower(&pcb, &config, false).unwrap().board;
        let net = board
            .nets
            .iter()
            .position(|net| net.name == "OPEN")
            .unwrap();
        board.classes[board.nets[net].class].layers = 1 << layers.index("In1.Cu").unwrap();
        let pads = [[2.0, 10.0], [18.0, 10.0]].map(|at| Endpoint {
            at,
            layers: layers.all(),
        });
        let scope = focus(&board, net, &[pads], None).unwrap();
        let proposal = search(
            &board,
            &config,
            &layers,
            "OPEN",
            parse(r#"(net "/OPEN")"#).unwrap(),
            scope,
            None,
        );
        assert_eq!(
            proposal.status,
            "all_route_layers_reserved_for_foreign_planes"
        );
        assert!(proposal.objects.is_empty());
        assert_eq!(proposal.search.unwrap().reserved_plane_layers, ["In1.Cu"]);
    }
}

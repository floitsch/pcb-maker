// Copyright (C) 2026 Florian Loitsch.

//! Short, jointly sized planar launches through a dense pad row.

use super::{Pad, Point, board_router, contact_on_net, item_id, pads, source_net_field, tips};
use crate::{
    board_router::KiCadBoardRouterConfig,
    kicad_nets::{net_codes, note_net_names},
    kicad_sexpr::{Expr, deterministic_uuid_for, encode, parse, quote},
};
use pcb_router::{
    Board, ObstacleKind, Shape, Terminal,
    geometry::{point_in_polygon, polygon_edges},
    rules::{ClassRules, NetRules, TRACK, obstacle_kind},
    verify::shape_distance,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_PADS: usize = 4;
const ROW_PITCH_LIMIT: f64 = 0.65;
const NM_PER_MM: f64 = 1_000_000.0;
const NM: f64 = 1.0 / NM_PER_MM;

#[derive(Clone, Debug, Serialize)]
pub struct PreparedRowEscape {
    pub pad: String,
    pub net: String,
    pub layer: String,
    pub width_mm: f64,
    pub start_mm: Point,
    pub end_mm: Point,
}

#[derive(Clone, Debug, Serialize)]
pub struct PreparedRowEscapeAdvice {
    pub router_config_patch: BTreeMap<&'static str, Vec<String>>,
    pub escapes: Vec<PreparedRowEscape>,
}

struct Port<'a> {
    pad: &'a Pad,
    terminal: &'a Terminal,
    net: usize,
    direction: Point,
    half_length: f64,
}

impl Port<'_> {
    fn end(&self, length: f64) -> Point {
        std::array::from_fn(|i| {
            ((self.terminal.anchor[i] + length * self.direction[i]) * NM_PER_MM).round() / NM_PER_MM
        })
    }

    fn axis(&self, length: f64) -> Shape {
        Shape::Capsule {
            start: self.terminal.anchor,
            end: self.end(length),
            radius: 0.0,
        }
    }
}

struct RowGeometry<'a> {
    board: &'a Board,
    rules: &'a ClassRules,
    layer: usize,
}

impl RowGeometry<'_> {
    fn class(&self, p: &Port<'_>) -> usize {
        self.board.nets[p.net].class
    }

    fn floor(&self, p: &Port<'_>) -> f64 {
        let class = self.class(p);
        self.board
            .neck_width
            .max(self.rules.floor_on(class, self.layer))
    }

    fn clearance(&self, a: &Port<'_>, b: &Port<'_>) -> f64 {
        let (a, b) = (self.class(a), self.class(b));
        self.board.classes[a]
            .clearance
            .max(self.board.classes[b].clearance)
            .max(self.rules.resolved.extra_on(
                self.rules.class_group[a],
                TRACK,
                self.rules.class_group[b],
                TRACK,
                self.layer,
            ))
    }

    /// Radius allowed at every point of this precise stub. The generic
    /// router's off-grid stub allowance is deliberately absent here:
    /// item-scoped pad clearances apply to these explicit pad contacts.
    fn width_for(&self, p: &Port<'_>, length: f64) -> f64 {
        let class = self.class(p);
        let axis = p.axis(length);
        let end = p.end(length);
        if !point_in_polygon(end, &self.board.outline) {
            return 0.0;
        }
        let mut width = self.rules.width_on(self.board, class, self.layer);
        for (a, b) in polygon_edges(&self.board.outline) {
            let border = Shape::Capsule {
                start: a,
                end: b,
                radius: 0.0,
            };
            width = width.min(2.0 * (shape_distance(&axis, &border) - self.board.edge_clearance));
        }
        for obstacle in &self.board.obstacles {
            if !obstacle.blocks_tracks || obstacle.layers & (1 << self.layer) == 0 {
                continue;
            }
            let clearance = match obstacle.kind {
                ObstacleKind::Copper => {
                    if obstacle.net == Some(p.net as pcb_router::NetId) {
                        continue;
                    }
                    let extra = obstacle_kind(obstacle).map_or(0.0, |kind| {
                        self.rules.resolved.extra_on(
                            self.rules.class_group[class],
                            TRACK,
                            self.rules.resolved.obstacle_group(obstacle),
                            kind,
                            self.layer,
                        )
                    });
                    self.board
                        .copper_clearance(&self.board.classes[class], obstacle)
                        .max(extra)
                }
                ObstacleKind::Hole => self.board.hole_clearance,
                ObstacleKind::Keepout => obstacle.clearance,
            };
            width = width.min(2.0 * (shape_distance(&axis, &obstacle.shape) - clearance));
            if width + NM < self.floor(p) {
                break;
            }
        }
        width
    }
}

/// A plan exists only for adjacent, distinctly named pads of one straight
/// row. Every requested port shares the widest balanced feasible launch;
/// neighboring ports have room for at least their native minimum width.
pub(crate) fn plan_row_escapes(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    requested: &[String],
) -> Result<Vec<(PreparedRowEscape, Expr)>, String> {
    if !(2..=MAX_PADS).contains(&requested.len())
        || requested.iter().collect::<BTreeSet<_>>().len() != requested.len()
    {
        return Err("prepared_row_escape_pads needs two to four distinct REF:PAD labels".into());
    }
    let Some((reference, _)) = requested[0].split_once(':') else {
        return Err("prepared row escapes need exact REF:PAD labels".into());
    };
    let mut footprints = pcb.children().iter().filter(|form| {
        form.head() == Some("footprint")
            && form.children().iter().any(|property| {
                property.head() == Some("property")
                    && property.children().get(1).and_then(Expr::atom) == Some("Reference")
                    && property.children().get(2).and_then(Expr::atom) == Some(reference)
            })
    });
    let (Some(footprint), None) = (footprints.next(), footprints.next()) else {
        return Err("prepared row escape footprint reference is unknown or ambiguous".into());
    };
    let mut lowering = config.clone();
    lowering.bga_fanout = Some(false);
    lowering.apply_width_floor();
    let lower = board_router::lower(pcb, &lowering, false)?;
    let fallback = NetRules::default();
    let (board, rules) = ClassRules::split(
        &lower.board,
        config.net_rules.as_deref().unwrap_or(&fallback),
    );
    let names = note_net_names(pcb);
    let pads = pads(pcb);
    let ports: Vec<_> = pads
        .iter()
        .filter(|pad| pad.reference == reference)
        .filter_map(|pad| {
            let (net, terminal) = contact_on_net(&board, &pad.label, names.normalize(&pad.net))?;
            if terminal.contact.is_some()
                || board.merged_pads.iter().any(|merged| {
                    merged.net as usize == net
                        && board.nets[net].terminals[merged.host].pad == terminal.pad
                })
            {
                return None;
            }
            let model_pad = footprint.children().iter().find(|item| {
                item.head() == Some("pad") && item_id(item).is_some_and(|id| id == pad.id)
            })?;
            let (direction, half_length) = tips::normal(model_pad, footprint, terminal.anchor)?;
            Some(Port {
                pad,
                terminal,
                net,
                direction,
                half_length,
            })
        })
        .collect();
    let Some(mut wanted) = requested
        .iter()
        .map(|label| {
            let (part, pin) = label.split_once(':')?;
            if part != reference {
                return None;
            }
            let mut matches = ports.iter().filter(|p| {
                p.pad
                    .label
                    .strip_prefix(reference)
                    .and_then(|rest| rest.strip_prefix('.'))
                    == Some(pin)
            });
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Err(
            "prepared row escapes require uniquely identified surface pads on one footprint".into(),
        );
    };
    if wanted.iter().map(|p| p.net).collect::<BTreeSet<_>>().len() != wanted.len() {
        return Err("prepared row escape pads must lead to distinct nets".into());
    }
    if wanted.iter().any(|port| {
        let net = &board.nets[port.net].name;
        config
            .length_rules
            .iter()
            .any(|rule| rule.nets.iter().any(|member| &member.net == net))
            || crate::project_rules::diff_pair_of(net).is_some_and(|(mate, _, positive)| {
                board.nets.iter().any(|net| net.name == mate)
                    && (config.diff_pairs_on()
                        || config
                            .diff_pair_rules
                            .get(if positive { net } else { &mate })
                            .is_some_and(|rule| {
                                rule.asked.is_some()
                                    || !rule.rules.is_empty()
                                    || rule.max_skew.is_some()
                                    || rule.length.is_some()
                                    || rule.disallowed.iter().any(|&disallowed| disallowed)
                            }))
            })
    }) {
        return Err(
            "prepared row escapes decline nets with authored length, skew or coupled routing rules"
                .into(),
        );
    }
    let first = wanted[0];
    let [dx, dy] = first.direction;
    let across = usize::from(dy.abs() > dx.abs());
    if first.direction[1 - across].abs() > NM {
        return Err("prepared row escapes require an axis-aligned pad row".into());
    }
    let row = |port: &Port<'_>| {
        port.terminal.layers == first.terminal.layers
            && (port.direction[0] - dx).abs() < NM
            && (port.direction[1] - dy).abs() < NM
            && (port.terminal.anchor[across] - first.terminal.anchor[across]).abs() < NM
    };
    let lateral = |port: &Port<'_>| port.terminal.anchor[1 - across];
    wanted.sort_by(|a, b| lateral(a).total_cmp(&lateral(b)));
    if !wanted.iter().all(|port| row(port))
        || wanted
            .windows(2)
            .any(|pair| lateral(pair[1]) - lateral(pair[0]) > ROW_PITCH_LIMIT + NM)
    {
        return Err("prepared row escapes require adjacent pads along one dense edge".into());
    }
    let lower = lateral(wanted[0]);
    let upper = lateral(wanted[wanted.len() - 1]);
    let neighbors: Vec<_> = ports
        .iter()
        .filter(|port| {
            row(port)
                && lateral(port) >= lower - ROW_PITCH_LIMIT
                && lateral(port) <= upper + ROW_PITCH_LIMIT
                && !wanted.iter().any(|wanted| wanted.pad.id == port.pad.id)
        })
        .collect();
    if neighbors
        .iter()
        .any(|port| (lower..=upper).contains(&lateral(port)))
    {
        return Err("prepared row escape pads must be consecutive within their row".into());
    }
    let layer = first.terminal.layers.trailing_zeros() as usize;
    let layer_name = if layer == 0 { "F.Cu" } else { "B.Cu" };
    if wanted.iter().any(|port| {
        let net = &board.nets[port.net];
        let allowed = board.classes[net.class].layers;
        allowed != 0 && allowed & port.terminal.layers == 0
            || config
                .project_rules
                .as_ref()
                .and_then(|project| project.track_disallowed_layers.get(&net.name))
                .is_some_and(|layers| layers.iter().any(|layer| layer == layer_name))
    }) {
        return Err("the project's rules disallow a prepared row track on the pad side".into());
    }
    let geometry = RowGeometry {
        board: &board,
        rules: &rules,
        layer,
    };
    let pad_width_floor = wanted.iter().map(|p| geometry.floor(p)).fold(0.0, f64::max);
    let max_pad_half = wanted.iter().map(|p| p.half_length).fold(0.0, f64::max);
    let pitch = (upper - lower) / (wanted.len() - 1) as f64;
    // Reach roughly one row pitch beyond the land. Late transitions cost
    // a little more surface copper, so prefer the shortest full exit.
    let goal = ((max_pad_half + pitch + 0.05) * 10.0).ceil() / 10.0;
    let widths = |length: f64| {
        let mut width = wanted
            .iter()
            .map(|port| geometry.width_for(port, length))
            .fold(f64::INFINITY, f64::min);
        for (index, port) in wanted.iter().enumerate() {
            let axis = port.axis(length);
            for other in &wanted[..index] {
                width = width.min(
                    shape_distance(&axis, &other.axis(length)) - geometry.clearance(port, other),
                );
            }
            for neighbor in &neighbors {
                if neighbor.net != port.net {
                    let guard = neighbor.half_length + 0.25;
                    width = width.min(
                        2.0 * (shape_distance(&axis, &neighbor.axis(guard))
                            - geometry.clearance(port, neighbor))
                            - geometry.floor(neighbor),
                    );
                }
            }
        }
        ((width + 1e-10) * NM_PER_MM).floor() / NM_PER_MM
    };
    let Some((length, width)) = [goal, goal + 0.1, goal - 0.1]
        .into_iter()
        .filter(|&length| length >= max_pad_half + 0.25 && length <= 2.5)
        .find_map(|length| {
            let width = widths(length);
            (width > 0.0 && width + 1e-9 >= pad_width_floor).then_some((length, width))
        })
    else {
        return Err(
            "no jointly legal planar exit above the requested pads' trace width floors".into(),
        );
    };
    let legacy_nets = net_codes(pcb);
    wanted.into_iter().map(|port| {
        let pad = port.pad.label.strip_prefix(reference).and_then(|rest| rest.strip_prefix('.'))
            .ok_or("prepared pad reference changed")?;
        let start = port.terminal.anchor;
        let end = port.end(length);
        let [ax, ay] = start;
        let [bx, by] = end;
        let net = encode(&source_net_field(&legacy_nets, &port.pad.net));
        let quoted_layer = quote(layer_name);
        let fields = format!("(start {ax:.6} {ay:.6}) (end {bx:.6} {by:.6}) (width {width:.6}) (layer {quoted_layer}) {net}");
        let id = deterministic_uuid_for(&format!("prepared row escape {} {fields}", port.pad.id));
        let form = parse(&format!("(segment {fields} (uuid \"{id}\"))"))?;
        Ok((PreparedRowEscape {
            pad: format!("{reference}:{pad}"),
            net: port.pad.net.clone(),
                layer: layer_name.into(),
            width_mm: width,
            start_mm: start,
            end_mm: end,
        }, form))
    }).collect()
}

/// Native common case: the reported opens identify consecutive surface
/// pads, and the same planner can find a feasible exit on the fixed pose.
pub fn prepared_row_escape_advice(
    pcb: &Expr,
    report: &Value,
    config: &KiCadBoardRouterConfig,
) -> Option<PreparedRowEscapeAdvice> {
    let findings = report["unconnected_items"].as_array()?;
    if !(2..=MAX_PADS).contains(&findings.len()) || super::report_capped(report) {
        return None;
    }
    let pads = pads(pcb);
    let first_ids: BTreeSet<_> = findings.first()?["items"]
        .as_array()?
        .iter()
        .filter_map(|item| Some(item["uuid"].as_str()?.to_ascii_lowercase()))
        .collect();
    let references: BTreeSet<_> = pads
        .iter()
        .filter(|pad| first_ids.contains(&pad.id))
        .map(|pad| &pad.reference)
        .collect();
    let mut groups = references.into_iter().filter_map(|reference| {
        super::native_pad_hits(report, &pads, |pad| &pad.reference == reference)
    });
    let (Some(hits), None) = (groups.next(), groups.next()) else {
        return None;
    };
    let names = note_net_names(pcb);
    // A nearby plane or power pad can have a separate access problem.
    // Offer a group sharing the project's existing class assignment.
    let mut classes = BTreeMap::<Vec<String>, Vec<_>>::new();
    for (finding, pad) in findings.iter().zip(hits) {
        let class = config
            .project_rules
            .as_ref()
            .and_then(|rules| rules.net_classes.get(names.normalize(&pad.net)))
            .cloned()
            .unwrap_or_default();
        let number = pad.label.strip_prefix(&pad.reference)?.strip_prefix('.')?;
        let label = format!("{}:{number}", pad.reference);
        classes
            .entry(class)
            .or_default()
            .push((finding, pad, label));
    }
    let initial = super::bare_placement(pcb.clone());
    let mut advice = classes
        .into_values()
        .filter(|group| group.len() >= 2)
        .filter_map(|group| {
            let requested: Vec<_> = group.iter().map(|(_, _, label)| label.clone()).collect();
            let escapes: Vec<_> = plan_row_escapes(&initial, config, &requested)
                .ok()?
                .into_iter()
                .map(|(escape, _)| escape)
                .collect();
            for (finding, pad, label) in group {
                let position = finding["items"]
                    .as_array()?
                    .iter()
                    .find(|item| {
                        item["uuid"]
                            .as_str()
                            .is_some_and(|id| id.eq_ignore_ascii_case(&pad.id))
                    })?
                    .get("pos")?;
                let at = [position["x"].as_f64()?, position["y"].as_f64()?];
                let launch = escapes.iter().find(|escape| escape.pad == label)?;
                if pcb_router::geometry::distance(at, launch.start_mm) > 0.01 {
                    return None;
                }
            }
            Some(PreparedRowEscapeAdvice {
                router_config_patch: BTreeMap::from([("prepared_row_escape_pads", requested)]),
                escapes,
            })
        });
    match (advice.next(), advice.next()) {
        (Some(first), None) => Some(first),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_router::rules::{ClearanceRule, PAD, kind_bit};

    fn fixture() -> (Expr, KiCadBoardRouterConfig, Vec<String>) {
        let pcb = parse(
            r#"(kicad_pcb (version 20241229)
            (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (44 "Edge.Cuts" user))
            (gr_rect (start 0 0) (end 20 20) (stroke (width 0.05) (type solid))
                (fill none) (layer "Edge.Cuts"))
            (footprint "T:dense" (layer "F.Cu") (at 5 5) (property "Reference" "U1")
                (pad "1" smd rect (at -0.5 1.5) (size 0.27 0.84) (layers "F.Cu")
                    (net "audio_a") (uuid "11111111-1111-1111-1111-111111111111"))
                (pad "2" smd rect (at 0 1.5) (size 0.27 0.84) (layers "F.Cu")
                    (net "audio_b") (uuid "22222222-2222-2222-2222-222222222222"))
                (pad "3" smd rect (at 0.5 1.5) (size 0.27 0.84) (layers "F.Cu")
                    (net "GND") (uuid "33333333-3333-3333-3333-333333333333")))
            (footprint "T:load" (layer "F.Cu") (at 6 15) (property "Reference" "J1")
                (pad "1" smd rect (at -2 0) (size 0.8 0.6) (layers "F.Cu")
                    (net "audio_a") (uuid "44444444-4444-4444-4444-444444444444"))
                (pad "2" smd rect (at 0 0) (size 0.8 0.6) (layers "F.Cu")
                    (net "audio_b") (uuid "55555555-5555-5555-5555-555555555555"))
                (pad "3" smd rect (at 2 0) (size 0.8 0.6) (layers "F.Cu")
                    (net "GND") (uuid "66666666-6666-6666-6666-666666666666"))))"#,
        )
        .unwrap();
        let rules = NetRules {
            clearances: vec![ClearanceRule {
                first: Some(vec!["audio_a".into(), "audio_b".into()]),
                second: None,
                kinds: kind_bit(TRACK, TRACK),
                layers: 0,
                clearance: 0.3,
                except_pads: BTreeSet::new(),
            }],
            ..NetRules::default()
        };
        let config = KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.5,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            neck_width_mm: Some(0.15),
            net_rules: Some(rules.into()),
            ..KiCadBoardRouterConfig::default()
        };
        (pcb, config, vec!["U1:1".into(), "U1:2".into()])
    }

    #[test]
    fn joint_width_keeps_track_spacing_and_the_neighbor_exit() {
        let (pcb, mut config, labels) = fixture();
        let plan = plan_row_escapes(&pcb, &config, &labels).unwrap();
        assert_eq!(plan.len(), 2);
        assert!(plan.iter().all(|(escape, form)| {
            (escape.width_mm - 0.2).abs() < NM && form.head() == Some("segment")
        }));
        let mut rules = config.net_rules.as_deref().unwrap().clone();
        rules.clearances[0].kinds |= kind_bit(TRACK, PAD);
        config.net_rules = Some(rules.into());
        assert!(plan_row_escapes(&pcb, &config, &labels).is_err());
    }

    #[test]
    fn explicit_floor_and_nonconsecutive_selection_decline() {
        let (pcb, mut config, labels) = fixture();
        assert!(plan_row_escapes(&pcb, &config, &["U1:1".into(), "U1:3".into()]).is_err());
        let mut rules = config.net_rules.as_deref().unwrap().clone();
        rules.width_floor.insert("audio_b".into(), 0.21);
        config.net_rules = Some(rules.into());
        assert!(plan_row_escapes(&pcb, &config, &labels).is_err());
    }

    #[test]
    fn default_pair_placeholders_allow_a_row_but_an_authored_skew_rule_declines() {
        let (pcb, mut config, labels) = fixture();
        let pcb = parse(
            &encode(&pcb)
                .replace("audio_a", "audio_P")
                .replace("audio_b", "audio_N"),
        )
        .unwrap();
        let mut native_rules = config.net_rules.as_deref().unwrap().clone();
        native_rules.clearances[0].first = Some(vec!["audio_P".into(), "audio_N".into()]);
        config.net_rules = Some(native_rules.into());
        config.diff_pair_rules.insert(
            "audio_P".into(),
            board_router::KiCadDiffPairRule {
                partner: "audio_N".into(),
                layers: vec![(0.2, 0.25); 2],
                ..Default::default()
            },
        );
        assert!(plan_row_escapes(&pcb, &config, &labels).is_ok());
        config.diff_pair_rules.get_mut("audio_P").unwrap().max_skew = Some(0.5);
        assert!(plan_row_escapes(&pcb, &config, &labels).is_err());
    }

    #[test]
    fn layout_emits_the_tips_from_its_lowered_pose_without_changing_placement() {
        let (mut pcb, mut config, labels) = fixture();
        config.prepared_row_escape_pads = labels;
        let (lowered, _) = board_router::lower_for_route(&pcb, &config, false).unwrap();
        assert_eq!(board_router::count_tracks(&pcb), [0, 0, 0]);
        let report = board_router::emit_layout_row_escapes(
            &mut pcb,
            &lowered,
            &config,
            &["F.Cu".into(), "B.Cu".into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(board_router::count_tracks(&pcb), [2, 0, 0]);
        let used = report.retained_pad_tips.unwrap().used;
        assert_eq!(used.len(), 2);
        assert!(used.iter().all(|tip| {
            pcb.children().iter().any(|form| {
                form.head() == Some("segment")
                    && item_id(form).as_deref() == Some(tip.track_uuid.as_str())
            })
        }));
    }

    #[test]
    fn advice_targets_the_shared_class_and_requires_current_native_pad_positions() {
        use serde_json::json;

        let (pcb, mut config, labels) = fixture();
        config.project_rules = Some(crate::project_rules::KiCadProjectRulesReport {
            net_classes: BTreeMap::from([
                ("audio_a".into(), vec!["Audio".into()]),
                ("audio_b".into(), vec!["Audio".into()]),
                ("GND".into(), vec!["Ground".into()]),
            ]),
            ..Default::default()
        });
        let mut report = json!({
            "violations": [],
            "unconnected_items": [
                {"items": [{"uuid":"11111111-1111-1111-1111-111111111111","pos":{"x":4.5,"y":6.5}}]},
                {"items": [{"uuid":"22222222-2222-2222-2222-222222222222","pos":{"x":5.0,"y":6.5}}]},
                {"items": [{"uuid":"33333333-3333-3333-3333-333333333333","pos":{"x":5.5,"y":6.5}}]}
            ]
        });
        let advice = prepared_row_escape_advice(&pcb, &report, &config).unwrap();
        assert_eq!(
            advice.router_config_patch["prepared_row_escape_pads"],
            labels
        );
        report["unconnected_items"][0]["items"][0]["pos"]["x"] = json!(4.0);
        assert!(prepared_row_escape_advice(&pcb, &report, &config).is_none());
    }
}

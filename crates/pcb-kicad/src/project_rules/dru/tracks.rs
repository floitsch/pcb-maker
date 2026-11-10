// Copyright (C) 2026 Toit contributors.

//! The rule KiCad applies to one track (differential pairs, length and
//! skew rules, layers a rule closes to a net's tracks).

use super::Rule;
use super::condition::{Context, Item, Kind, NetFacts, T3, parse_condition};
use super::lower::rule_layers;

/// The values of the one rule KiCad applies to an item for a constraint
/// (`track_constraint`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Picked {
    pub rule: String,
    pub min: Option<f64>,
    pub opt: Option<f64>,
    pub max: Option<f64>,
}

/// Whether a custom rule surely disallows a track of `net` on `layer`
/// (`(constraint disallow track)` with a condition that surely holds;
/// ducktop2's "solid ground layers": no track but GND's on In1.Cu).
pub(crate) fn track_disallowed(
    rules: &[Rule],
    net: &NetFacts,
    layer: &str,
    copper: &[String],
) -> bool {
    let item = Item {
        kind: Kind::Track,
        net: Some(net),
        layer,
    };
    rules.iter().any(|rule| {
        rule.severity.as_deref() != Some("ignore")
            && rule_layers(rule, copper).contains(&layer)
            && rule.constraints.iter().any(|constraint| {
                constraint.kind == "disallow" && constraint.words.iter().any(|word| word == "track")
            })
            && match rule.condition.as_deref() {
                None => true,
                Some(text) => parse_condition(text).is_ok_and(|condition| {
                    Context {
                        a: item,
                        b: None,
                        same_net: false,
                    }
                    .truth(&condition)
                        == T3::True
                }),
            }
    })
}

/// The rule KiCad applies to a track of `net` on `layer` for a constraint
/// `kind` checked on the item alone (track width, diff pair gap, uncoupled
/// length): the last rule whose `(layer ...)` holds there and whose
/// condition surely holds for it (`EvalRules`: the last match wins).
/// Rules whose condition is unknown for the track, that do not parse or
/// are rated `ignore` are passed over.
pub(crate) fn track_constraint(
    rules: &[Rule],
    net: &NetFacts,
    layer: &str,
    copper: &[String],
    kind: &str,
) -> Option<Picked> {
    track_rule(rules, net, layer, copper, kind).map(|(_, picked)| picked)
}

/// `track_constraint` with the rule's index in `rules`: KiCad groups the
/// items of a `length` or `skew` constraint by the rule that applies to
/// them (two rules of one name are two groups).
pub(crate) fn track_rule(
    rules: &[Rule],
    net: &NetFacts,
    layer: &str,
    copper: &[String],
    kind: &str,
) -> Option<(usize, Picked)> {
    let item = Item {
        kind: Kind::Track,
        net: Some(net),
        layer,
    };
    rules.iter().enumerate().rev().find_map(|(index, rule)| {
        if rule.severity.as_deref() == Some("ignore") || !rule_layers(rule, copper).contains(&layer)
        {
            return None;
        }
        let constraint = rule
            .constraints
            .iter()
            .rev()
            .find(|constraint| constraint.kind == kind)?;
        let holds = match rule.condition.as_deref() {
            None => T3::True,
            Some(text) => {
                let condition = parse_condition(text).ok()?;
                Context {
                    a: item,
                    b: None,
                    same_net: false,
                }
                .truth(&condition)
            }
        };
        (holds == T3::True).then(|| {
            (
                index,
                Picked {
                    rule: rule.name.clone(),
                    min: constraint.min,
                    opt: constraint.opt,
                    max: constraint.max,
                },
            )
        })
    })
}

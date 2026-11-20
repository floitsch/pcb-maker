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

/// Whether a custom coupling or length rule could hold for either member
/// of a pair. A condition whose item geometry is unknown stays in scope.
pub(crate) fn may_constrain_pair(rules: &[Rule], nets: [&NetFacts; 2], copper: &[String]) -> bool {
    rules.iter().any(|rule| {
        if rule.severity.as_deref() == Some("ignore")
            || !rule.constraints.iter().any(|constraint| {
                matches!(
                    constraint.kind.as_str(),
                    "diff_pair_gap" | "diff_pair_uncoupled" | "skew" | "length"
                )
            })
        {
            return false;
        }
        let layers = rule_layers(rule, copper);
        if layers.is_empty() {
            return false;
        }
        let Some(text) = rule.condition.as_deref() else {
            return true;
        };
        let Ok(condition) = parse_condition(text) else {
            return true;
        };
        for (index, net) in nets.iter().enumerate() {
            for &layer in &layers {
                for kind in [Kind::Track, Kind::Via] {
                    let a = Item {
                        kind,
                        net: Some(net),
                        layer,
                    };
                    let partner = |kind| {
                        Some(Item {
                            kind,
                            net: Some(nets[1 - index]),
                            ..a
                        })
                    };
                    if [None, partner(Kind::Track), partner(Kind::Via)]
                        .into_iter()
                        .any(|b| {
                            (Context {
                                a,
                                b,
                                same_net: false,
                            })
                            .truth(&condition)
                                != T3::False
                        })
                    {
                        return true;
                    }
                }
            }
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_pair_policy_checks_both_names_and_protects_unresolved_custom_scope() {
        let names = ["/LINK+", "/LINK-"].map(|name| NetFacts {
            name: name.into(),
            classes: vec!["Default".into()],
            diff_pair: Some(Some("/LINK".into())),
        });
        let layers = ["F.Cu".into(), "B.Cu".into()];
        let check = |rule| {
            let rules = super::super::parse_rules(rule).unwrap();
            may_constrain_pair(&rules, [&names[0], &names[1]], &layers)
        };
        assert!(!check(
            r#"(version 1) (rule "Other pair" (condition "A.NetName == '/OTHER+'") (constraint diff_pair_uncoupled (max 4mm)))"#
        ));
        assert!(check(
            r#"(version 1) (rule "Companion gap" (condition "A.NetName == '/LINK+'") (constraint diff_pair_gap (max 0.5mm)))"#
        ));
        assert!(check(
            r#"(version 1) (rule "Pair skew" (condition "A.inDiffPair('/LINK')") (constraint skew (max 0.1mm)))"#
        ));
        assert!(check(
            r#"(version 1) (rule "Local length" (condition "A.intersectsArea('critical')") (constraint length (max 100mm)))"#
        ));
        assert!(!check(
            r#"(version 1) (rule "Track width" (condition "A.NetName == '/LINK+'") (constraint track_width (min 0.4mm)))"#
        ));
    }
}

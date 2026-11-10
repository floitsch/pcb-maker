// Copyright (C) 2026 Toit contributors.

//! Skew groups' layers (docs/router.md, "Skew groups' layers").
//!
//! KiCad counts a net's copper for a length or skew rule with a layer
//! condition on those layers alone, so a net routed elsewhere is not
//! checked by the rule and leaves its skew group, and a net routed onto
//! them joins it (trial 7's Sisu_SSE-9: a 0.5 mm skew over the 11 USB
//! nets on In1.Cu and In4.Cu; the designer runs MODEM_USB there and the
//! other USB nets on the outer layers, the route put four more nets on
//! the inner layers, where they could not be matched to MODEM_USB's 42
//! mm). Each net of such a rule gets the layers it prefers
//! (`KiCadBoardRouterConfig::length_rule_layers`): the side of the rule's
//! layers the designer's board routes it on, or the rule's layers. The
//! router prices a step off them (`pcb_router::Config::layer_preferences`).

use super::*;

/// The coupled links of a pair given its nets' preference
/// (`core_layer_preferences`); a pair has one link between each two of
/// its points side by side, seldom more than a handful.
const MAX_LINKS: usize = 32;

/// Per net of a layer-limited `length` or `skew` rule, the layers it
/// prefers (`KiCadBoardRouterConfig::length_rule_layers`). `copper`: the
/// board's copper layers in the router's order; `closed`: per net the
/// layers a `disallow track` rule closes; `designer`: the designer's
/// tracks per net and layer (`class_view::track_lengths`), if any.
/// A net of several such rules prefers the layers all of them prefer
/// (none where they disagree).
pub(crate) fn length_layer_preferences(
    rules: &[KiCadLengthRule],
    mode: &str,
    copper: &[String],
    closed: &BTreeMap<String, pcb_router::LayerMask>,
    designer: Option<&BTreeMap<String, BTreeMap<String, f64>>>,
) -> BTreeMap<String, KiCadLayerPreference> {
    let mut preferences: BTreeMap<String, KiCadLayerPreference> = BTreeMap::new();
    if mode == "off" || copper.is_empty() || copper.len() > 32 {
        return preferences;
    }
    let all: pcb_router::LayerMask = if copper.len() == 32 {
        pcb_router::LayerMask::MAX
    } else {
        (1 << copper.len()) - 1
    };
    let mask_of = |names: &[String]| -> pcb_router::LayerMask {
        copper
            .iter()
            .enumerate()
            .filter(|(_, name)| names.contains(name))
            .fold(0, |mask, (layer, _)| mask | (1 << layer))
    };
    let names_of = |mask: pcb_router::LayerMask| -> Vec<String> {
        copper
            .iter()
            .enumerate()
            .filter(|(layer, _)| mask & (1 << layer) != 0)
            .map(|(_, name)| name.clone())
            .collect()
    };
    let mut disagreeing: BTreeSet<String> = BTreeSet::new();
    for rule in rules {
        for member in &rule.nets {
            let ruled = mask_of(&member.layers);
            if ruled == 0 || ruled == all {
                continue;
            }
            let open = all & !closed.get(&member.net).copied().unwrap_or(0);
            let (on, source) = if mode == "rule" {
                (true, "the rule")
            } else {
                let Some(tracks) = designer.and_then(|designer| designer.get(&member.net)) else {
                    continue;
                };
                let on = tracks
                    .iter()
                    .any(|(layer, length)| *length > 0.0 && member.layers.contains(layer));
                (on, "the designer's board")
            };
            let mask = if on { ruled & open } else { !ruled & open };
            if mask == 0 || mask == open {
                continue;
            }
            match preferences.get_mut(&member.net) {
                Some(earlier) if earlier.mask & mask == 0 => {
                    disagreeing.insert(member.net.clone());
                }
                Some(earlier) => {
                    earlier.mask &= mask;
                    earlier.layers = names_of(earlier.mask);
                }
                None => {
                    preferences.insert(
                        member.net.clone(),
                        KiCadLayerPreference {
                            mask,
                            layers: names_of(mask),
                            on_rule_layers: on,
                            source: source.to_string(),
                        },
                    );
                }
            }
        }
    }
    for net in disagreeing {
        preferences.remove(&net);
    }
    preferences
}

/// Sets `config.length_layer_preferences` for the board `input` (its
/// copper layers) from the project's length and skew rules and the first
/// of `designer_boards` (the reference's board, then the input's) with
/// tracks of a net of such a rule; nothing without a layer-limited rule
/// or with `length_matching` off.
pub fn admit_length_rule_layers(
    config: &mut KiCadBoardRouterConfig,
    input: &Path,
    designer_boards: &[PathBuf],
) {
    config.length_layer_preferences.clear();
    if config.length_rules.is_empty() || !config.length_matching_on() {
        return;
    }
    let mode = config.length_rule_layers_mode();
    if mode == "off" {
        return;
    }
    let Some(copper) = fs::read_to_string(input)
        .ok()
        .and_then(|text| parse(&text).ok())
        .map(|pcb| crate::project_rules::copper_layer_names(&pcb))
    else {
        return;
    };
    let limited = config.length_rules.iter().any(|rule| {
        rule.nets
            .iter()
            .any(|member| !member.layers.is_empty() && member.layers.len() < copper.len())
    });
    if !limited {
        return;
    }
    let members: BTreeSet<&str> = config
        .length_rules
        .iter()
        .flat_map(|rule| rule.nets.iter().map(|member| member.net.as_str()))
        .collect();
    let designer = if mode == "designer" {
        designer_boards.iter().find_map(|path| {
            let text = fs::read_to_string(path).ok()?;
            let lengths = crate::class_view::track_lengths(&parse(&text).ok()?);
            lengths
                .keys()
                .any(|net| members.contains(net.as_str()))
                .then_some(lengths)
        })
    } else {
        None
    };
    config.length_layer_preferences = length_layer_preferences(
        &config.length_rules,
        mode,
        &copper,
        &config.track_disallowed,
        designer.as_ref(),
    );
    let side = |on: bool| -> Vec<&str> {
        config
            .length_layer_preferences
            .iter()
            .filter(|(_, preference)| preference.on_rule_layers == on)
            .map(|(net, _)| net.as_str())
            .collect()
    };
    let (on, off) = (side(true), side(false));
    eprintln!(
        "length rules: {} nets of rules on some layers only prefer a side of them (length_rule_layers \"{mode}\"{}): on the rule's layers {}; off them {}",
        on.len() + off.len(),
        if mode == "designer" && designer.is_none() {
            ", no designer's board with their tracks"
        } else {
            ""
        },
        if on.is_empty() {
            "none".into()
        } else {
            on.join(", ")
        },
        if off.is_empty() {
            "none".into()
        } else {
            off.join(", ")
        },
    );
}

/// The core router's `layer_preferences` and their cost from
/// `config.length_layer_preferences` (`core_config`).
pub(crate) fn core_layer_preferences(
    config: &KiCadBoardRouterConfig,
    router_config: &mut pcb_router::Config,
) {
    if config.length_layer_preferences.is_empty() || !config.length_matching_on() {
        return;
    }
    let mut masks: BTreeMap<String, pcb_router::LayerMask> = config
        .length_layer_preferences
        .iter()
        .map(|(net, preference)| (net.clone(), preference.mask))
        .collect();
    // A differential pair whose two nets prefer the same layers: its
    // coupled links (nets of their own on the lowered board,
    // `diff_pairs::link_name`) prefer them too (trial 7's Sisu_SSE-9:
    // RP_USB and MODEM_USB_PHY, off In1.Cu and In4.Cu on the designer's
    // board, coupled on In4.Cu and joined the skew group).
    for (net, preference) in &config.length_layer_preferences {
        let Some((partner, _, true)) = crate::project_rules::diff_pair_of(net) else {
            continue;
        };
        if config
            .length_layer_preferences
            .get(&partner)
            .is_some_and(|other| other.mask == preference.mask)
        {
            for number in 1..=MAX_LINKS {
                masks.insert(
                    super::diff_pairs::link_name(net, &partner, number),
                    preference.mask,
                );
            }
        }
    }
    router_config.layer_preferences = Some(std::sync::Arc::new(masks));
    if let Some(cost) = config.layer_preference_cost {
        router_config.layer_preference_cost = cost.max(1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(nets: &[(&str, &[&str])]) -> KiCadLengthRule {
        KiCadLengthRule {
            rule: "USB".into(),
            constraint: "skew".into(),
            max: Some(0.5),
            nets: nets
                .iter()
                .map(|(net, layers)| KiCadLengthMember {
                    net: net.to_string(),
                    layers: layers.iter().map(|layer| layer.to_string()).collect(),
                    vias: false,
                })
                .collect(),
            ..KiCadLengthRule::default()
        }
    }

    fn copper() -> Vec<String> {
        ["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]
            .iter()
            .map(|name| name.to_string())
            .collect()
    }

    #[test]
    fn a_skew_groups_nets_prefer_the_side_of_its_layers_the_designer_routes_them_on() {
        // Sisu_SSE-9's shape: a skew rule on In1.Cu, In2.Cu closed to the
        // class; the designer runs LONG on In1.Cu (and a bit of F.Cu),
        // SHORT on F.Cu only, and has no tracks of NEW.
        let inner: &[&str] = &["In1.Cu"];
        let rules = vec![rule(&[
            ("LONG", inner),
            ("SHORT", inner),
            ("NEW", inner),
            ("EVERY", &["F.Cu", "In1.Cu", "In2.Cu", "B.Cu"]),
        ])];
        let closed: BTreeMap<String, pcb_router::LayerMask> = ["LONG", "SHORT", "NEW"]
            .iter()
            .map(|net| (net.to_string(), 0b0100))
            .collect();
        let designer: BTreeMap<String, BTreeMap<String, f64>> = [
            ("LONG", vec![("In1.Cu", 72.0), ("F.Cu", 4.0)]),
            ("SHORT", vec![("F.Cu", 7.0)]),
            ("EVERY", vec![("B.Cu", 3.0)]),
        ]
        .into_iter()
        .map(|(net, layers)| {
            (
                net.to_string(),
                layers
                    .into_iter()
                    .map(|(layer, length)| (layer.to_string(), length))
                    .collect(),
            )
        })
        .collect();
        let preferences =
            length_layer_preferences(&rules, "designer", &copper(), &closed, Some(&designer));
        assert_eq!(
            preferences["LONG"],
            KiCadLayerPreference {
                mask: 0b0010,
                layers: vec!["In1.Cu".into()],
                on_rule_layers: true,
                source: "the designer's board".into(),
            }
        );
        // Off the rule's layers: the open ones (In2.Cu closed).
        assert_eq!(preferences["SHORT"].mask, 0b1001);
        assert_eq!(preferences["SHORT"].layers, ["F.Cu", "B.Cu"]);
        assert!(!preferences["SHORT"].on_rule_layers);
        // No designer's tracks, a rule on every layer: nothing.
        assert!(!preferences.contains_key("NEW") && !preferences.contains_key("EVERY"));
        // The rule's own way: every net of it on its layers.
        let ruled = length_layer_preferences(&rules, "rule", &copper(), &closed, None);
        assert_eq!(
            ruled
                .iter()
                .map(|(net, preference)| (net.as_str(), preference.mask))
                .collect::<Vec<_>>(),
            [("LONG", 0b0010), ("NEW", 0b0010), ("SHORT", 0b0010)]
        );
        assert!(
            ruled
                .values()
                .all(|preference| preference.source == "the rule")
        );
        assert!(
            length_layer_preferences(&rules, "off", &copper(), &closed, Some(&designer)).is_empty()
        );
        // The core router gets the masks, the settings' cost floored at 1.
        let mut config = KiCadBoardRouterConfig {
            length_layer_preferences: preferences,
            layer_preference_cost: Some(0.5),
            ..KiCadBoardRouterConfig::default()
        };
        let core = core_config(&config);
        assert_eq!(
            core.layer_preferences.as_deref().map(|masks| masks.len()),
            Some(2)
        );
        assert_eq!(core.layer_preference_cost, 1.0);
        config.length_matching = Some(false);
        assert!(core_config(&config).layer_preferences.is_none());
    }

    #[test]
    fn rules_that_disagree_on_a_net_give_it_no_preference() {
        let rules = vec![rule(&[("A", &["In1.Cu"])]), rule(&[("A", &["In2.Cu"])])];
        let preferences =
            length_layer_preferences(&rules, "rule", &copper(), &BTreeMap::new(), None);
        assert!(preferences.is_empty(), "{preferences:?}");
        let agreeing = vec![
            rule(&[("A", &["In1.Cu", "In2.Cu"])]),
            rule(&[("A", &["In1.Cu"])]),
        ];
        let preferences =
            length_layer_preferences(&agreeing, "rule", &copper(), &BTreeMap::new(), None);
        assert_eq!(preferences["A"].layers, ["In1.Cu"]);
    }
}

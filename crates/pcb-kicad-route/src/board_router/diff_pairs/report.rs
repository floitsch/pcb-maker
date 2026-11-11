// Copyright (C) 2026 Toit contributors.

//! What the differential pairs came to: `board-router.json`'s and the
//! verdict's `diff_pairs`.

use super::*;

/// What the differential pairs came to (`board-router.json`'s
/// `diff_pairs`, the verdict's).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadDiffPairs {
    /// Whether the pairs routed coupled (`diff_pairs`); off, the counts
    /// below are 0 (no pair was planned).
    #[serde(default = "coupled_by_default")]
    pub on: bool,
    /// Why: "explicit" (the settings' `diff_pairs` or
    /// `PCB_ROUTER_DIFF_PAIRS`), "rules" (the project's custom rules ask
    /// for pairs: route mode's default by need), "off" (nothing asks).
    #[serde(default)]
    pub default: String,
    /// The setting, or the rule that asks and the net it holds for, or why
    /// nothing asks.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    /// Pairs found by name (both nets with pads on the board).
    pub found: usize,
    /// Pairs whose every link is coupled.
    pub coupled: usize,
    /// Pairs with some links coupled, not all.
    pub partly_coupled: usize,
    /// Pairs with no link coupled: routed as two ordinary nets.
    pub fallen_back: usize,
    /// Links between pair points (of different parts), and those coupled.
    pub links: usize,
    pub links_coupled: usize,
    /// Coupled links whose pads' order crosses the run's at one end (the
    /// router routes the stubs there, crossing with a via).
    pub crossed: usize,
    /// Of those, the links whose crossed end has a crossover in its lead:
    /// one net dives under the other with a via on each side of it, so the
    /// stubs do not cross (docs/router.md, "Polarity").
    #[serde(default)]
    pub crossovers: usize,
    /// The coupled runs' length (millimetres, along the positive net).
    pub coupled_mm: f64,
    /// The coupled pairs' length beyond their coupled runs, after routing:
    /// per pair the longer net's track length less its coupled length
    /// (KiCad's uncoupled length), summed.
    pub uncoupled_mm: f64,
    /// The links whose run was not routed, by cause (`not_routed_cause`:
    /// "a launch has no legal node", "no path in the negotiation", "the
    /// hard repair found no room", "no hard repair: the attempt was judged
    /// hopeless", "the hard repair's budget ran out before it", "lost
    /// after the repair").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub not_routed: BTreeMap<String, usize>,
    /// Of those, the links the negotiation gave up (`core::GiveUp`;
    /// docs/router.md, "Giving up on a link"): their pairs' nets routed
    /// uncoupled from then on, in the same attempt. Their cause in
    /// `not_routed` is "given up in the negotiation: no room" or "...: no
    /// path".
    #[serde(default)]
    pub given_up: usize,
    /// Stubs routed after the attempt for links whose copper holds but
    /// whose pads' stubs did not reach the run (`repair_stubs`; before
    /// 2026-11-05 such a link fell back: "a pad's stub does not reach the
    /// run"). Their work is in `work_seconds`.
    #[serde(default, skip_serializing_if = "is_no_count")]
    pub stubs_routed: usize,
    /// The fallback router's and the stub repair's work (seconds of an
    /// idle machine), charged to the ladder.
    pub work_seconds: f64,
    /// The largest skew of a pair with a coupled link (its nets' lengths
    /// apart after the route, from the written copper: tracks, and vias at
    /// the stackup's heights; docs/router.md, "Skew and length").
    #[serde(default)]
    pub skew_mm: f64,
    /// Meanders added to match the pairs' lengths (`KiCadPairSkew`).
    #[serde(default)]
    pub meanders: usize,
    /// The kept links' vias (both nets' at each layer change) and the
    /// return vias of the reference net placed beside them
    /// (`place_return_vias`; docs/router.md, "Return vias").
    #[serde(default)]
    pub pair_vias: usize,
    #[serde(default)]
    pub return_vias: usize,
    /// The reference net of the return vias, where they are on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub return_net: String,
    /// Per pair with a coupled link: its lengths and skew (kept in the
    /// verdict, unlike `pairs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skews: Vec<KiCadPairSkew>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pairs: Vec<KiCadDiffPair>,
}

/// One coupled pair's lengths (`KiCadDiffPairs::skews`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadPairSkew {
    pub positive: String,
    pub negative: String,
    /// The two nets' lengths (positive, negative), millimetres.
    pub lengths_mm: [f64; 2],
    /// The longer net's length less the coupled run's (KiCad's uncoupled
    /// length: the stubs and the router's ways to the pads).
    #[serde(default)]
    pub uncoupled_mm: f64,
    pub skew_mm: f64,
    /// The skew the pair is matched to: the custom rules' `skew` maximum,
    /// else the pair's gap.
    pub target_mm: f64,
    /// What the meander added to the shorter net (0: none).
    pub meander_mm: f64,
    /// Its kept links' vias and the return vias beside them.
    #[serde(default)]
    pub pair_vias: usize,
    #[serde(default)]
    pub return_vias: usize,
}

fn is_no_count(count: &usize) -> bool {
    *count == 0
}

/// Reports from before the switch's default by need were made with the
/// switch on.
fn coupled_by_default() -> bool {
    true
}

/// One differential pair (`KiCadDiffPairs::pairs`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct KiCadDiffPair {
    pub positive: String,
    pub negative: String,
    pub width_mm: f64,
    pub gap_mm: f64,
    /// Where width and gap came from (a custom rule, the net class).
    pub source: String,
    pub links: usize,
    pub links_coupled: usize,
    pub coupled_mm: f64,
    /// After routing: the longer net's length less the coupled length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncoupled_mm: Option<f64>,
    /// Why links fell back, when some did.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<String>,
    /// After routing: the two nets' lengths apart (`KiCadPairSkew`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_mm: Option<f64>,
    /// The skew it is matched to and what the meander added.
    #[serde(default)]
    pub skew_target_mm: f64,
    #[serde(default)]
    pub meander_mm: f64,
    /// Its kept links' vias and the return vias beside them
    /// (`place_return_vias`).
    #[serde(default)]
    pub pair_vias: usize,
    #[serde(default)]
    pub return_vias: usize,
}

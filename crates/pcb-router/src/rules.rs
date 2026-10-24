// Copyright (C) 2026 Toit contributors.
// Use of this source code is governed by an MIT-style license that can be
// found in the LICENSE file.

//! Rules beyond the net classes (`Config::net_rules`): a width floor per
//! net, track widths per layer, clearances between two sets of nets, per
//! item type and per layer, and the hole-to-hole spacing of a via from a
//! pad hole of its own net.
//!
//! The net classes give each net one clearance, and a pair of nets keeps
//! the larger of the two. A clearance rule here raises that for the pairs
//! it names: an item of a net of `first` (a track or a via) against an
//! item of a net of `second` (a track, a via or a pad), on the layers it
//! names. The router splits its rule classes so that the nets a rule
//! tells apart have classes of their own (their `group`), and builds its
//! stamps and static maps per pair of classes from `Resolved::extra`.

use std::collections::BTreeMap;

use crate::board::{Board, Obstacle, ObstacleKind, RuleClass};

/// Item kinds of a clearance rule.
pub const TRACK: usize = 0;
pub const VIA: usize = 1;
pub const PAD: usize = 2;

/// The bit of `ClearanceRule::kinds` for an item of kind `first` (TRACK or
/// VIA) of the rule's first side against one of kind `second` (TRACK,
/// VIA or PAD) of its second.
pub const fn kind_bit(first: usize, second: usize) -> u16 {
    1 << (first * 3 + second)
}

/// Every pair of kinds.
pub const ALL_KINDS: u16 = 0b111_111;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetRules {
    /// Per net (by name): the narrowest track it may have, also in its
    /// neck-downs and in the narrow-signal step (KiCad's `track_width
    /// min` of the rules that match it). A net without an entry keeps the
    /// board's neck width.
    pub width_floor: BTreeMap<String, f64>,
    pub clearances: Vec<ClearanceRule>,
    /// The hole-to-hole spacing a via keeps from a pad hole of its own net
    /// (KiCad's hole_to_hole holds within a net too); 0: none, a via of a
    /// pad's net keeps only the pad copper's rule.
    pub same_net_hole_to_hole: f64,
    /// Per net (by name): its tracks' width bounds per router layer, where
    /// the custom rules bound them differently on different layers (KiCad's
    /// `track_width` with a `(layer ...)` clause or a layer condition:
    /// OpenESC's 0.16 mm on F.Cu and B.Cu over the board's 0.09). The net
    /// routes on each layer at its class's width held inside that layer's
    /// bounds, and necks down no further than the layer's minimum. A net
    /// without an entry has one width on every layer.
    pub layer_widths: BTreeMap<String, Vec<LayerWidth>>,
}

impl NetRules {
    /// Whether the rules change anything.
    pub fn is_empty(&self) -> bool {
        self.width_floor.is_empty()
            && self.clearances.is_empty()
            && self.same_net_hole_to_hole <= 0.0
            && self.layer_widths.is_empty()
    }
}

/// A net's track width bounds on one layer (`NetRules::layer_widths`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerWidth {
    /// The narrowest track on the layer (its neck-downs too); 0: none.
    pub min: f64,
    /// The widest; infinity: none.
    pub max: f64,
    /// The width to route at on the layer (KiCad's opt); `None`: the
    /// class's.
    pub opt: Option<f64>,
}

impl LayerWidth {
    /// No bound.
    pub const ANY: Self = Self {
        min: 0.0,
        max: f64::INFINITY,
        opt: None,
    };

    /// The width a track of a class of `width` has on the layer.
    pub fn width(&self, width: f64) -> f64 {
        self.opt.unwrap_or(width).min(self.max).max(self.min)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClearanceRule {
    /// The nets of the first side by name; `None`: every item, those of
    /// no net too.
    pub first: Option<Vec<String>>,
    pub second: Option<Vec<String>>,
    /// The pairs of item kinds (`kind_bit`). A pair holds both ways round:
    /// a track of the first side against a pad of the second is the pad
    /// against the track.
    pub kinds: u16,
    /// The copper layers (bit per router layer); 0: every layer.
    pub layers: u32,
    pub clearance: f64,
}

impl ClearanceRule {
    fn on_layer(&self, layer: usize) -> bool {
        self.layers == 0 || self.layers & (1 << layer) != 0
    }
}

/// The rules resolved against a board: nets in groups (the nets no rule
/// tells apart), and per pair of groups, kinds and layer variant the
/// clearance the rules ask beyond the classes (0: none).
#[derive(Clone, Debug)]
pub struct Resolved {
    pub groups: usize,
    /// Per `NetId`.
    pub net_group: Vec<usize>,
    /// The group of items of no net.
    pub no_net_group: usize,
    /// Per layer: its variant (layers the same rules hold on, with the same
    /// track widths, share one).
    pub layer_variant: Vec<usize>,
    pub variants: usize,
    /// Per variant: its first layer (where its widths are read).
    pub variant_layer: Vec<usize>,
    /// Per `NetId`: its width bounds per layer (`NetRules::layer_widths`),
    /// empty for a net with one width on every layer.
    pub layer_widths: Vec<Vec<LayerWidth>>,
    /// Per `NetId`: its width floor (0: none).
    pub width_floor: Vec<f64>,
    pub same_net_hole_to_hole: f64,
    /// `[((a * groups + b) * 9 + kind_a * 3 + kind_b) * variants + variant]`.
    table: Vec<f64>,
}

impl Resolved {
    pub fn new(rules: &NetRules, board: &Board) -> Self {
        let layers = board.layer_count;
        // Which side of which rule a net is on.
        let member = |side: &Option<Vec<String>>, name: Option<&str>| match (side, name) {
            (None, _) => true,
            (Some(names), Some(name)) => names.iter().any(|known| known == name),
            (Some(_), None) => false,
        };
        let signature = |name: Option<&str>| -> Vec<bool> {
            rules
                .clearances
                .iter()
                .flat_map(|rule| [member(&rule.first, name), member(&rule.second, name)])
                .collect()
        };
        let mut signatures: Vec<Vec<bool>> = Vec::new();
        let mut group_of = |signature: Vec<bool>| {
            signatures
                .iter()
                .position(|known| *known == signature)
                .unwrap_or_else(|| {
                    signatures.push(signature);
                    signatures.len() - 1
                })
        };
        let net_group: Vec<usize> = board
            .nets
            .iter()
            .map(|net| group_of(signature(Some(&net.name))))
            .collect();
        let no_net_group = group_of(signature(None));
        let groups = signatures.len();

        let layer_widths: Vec<Vec<LayerWidth>> = board
            .nets
            .iter()
            .map(|net| {
                rules
                    .layer_widths
                    .get(&net.name)
                    .map(|widths| {
                        (0..layers)
                            .map(|layer| widths.get(layer).copied().unwrap_or(LayerWidth::ANY))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        // Layers share a variant where the same clearance rules hold and
        // every net has the same width bounds.
        let mut layer_signatures: Vec<(Vec<bool>, Vec<u64>)> = Vec::new();
        let mut variant_layer = Vec::new();
        let layer_variant: Vec<usize> = (0..layers)
            .map(|layer| {
                let signature: Vec<bool> = rules
                    .clearances
                    .iter()
                    .map(|rule| rule.on_layer(layer))
                    .collect();
                let widths: Vec<u64> = layer_widths
                    .iter()
                    .filter(|widths| !widths.is_empty())
                    .flat_map(|widths| {
                        let bounds = widths[layer];
                        [
                            bounds.min.to_bits(),
                            bounds.max.to_bits(),
                            bounds.opt.map_or(u64::MAX, f64::to_bits),
                        ]
                    })
                    .collect();
                let signature = (signature, widths);
                layer_signatures
                    .iter()
                    .position(|known| *known == signature)
                    .unwrap_or_else(|| {
                        layer_signatures.push(signature);
                        variant_layer.push(layer);
                        layer_signatures.len() - 1
                    })
            })
            .collect();
        if variant_layer.is_empty() {
            variant_layer.push(0);
        }
        let variants = layer_signatures.len().max(1);
        let mut table = vec![0.0f64; groups * groups * 9 * variants];
        for (index, rule) in rules.clearances.iter().enumerate() {
            let firsts: Vec<usize> = (0..groups)
                .filter(|group| signatures[*group][2 * index])
                .collect();
            let seconds: Vec<usize> = (0..groups)
                .filter(|group| signatures[*group][2 * index + 1])
                .collect();
            for variant in 0..variants {
                if !layer_signatures
                    .get(variant)
                    .is_some_and(|signature| signature.0[index])
                {
                    continue;
                }
                for &a in &firsts {
                    for &b in &seconds {
                        for kind_a in [TRACK, VIA] {
                            for kind_b in [TRACK, VIA, PAD] {
                                if rule.kinds & kind_bit(kind_a, kind_b) == 0 {
                                    continue;
                                }
                                for (g, h, k, l) in [(a, b, kind_a, kind_b), (b, a, kind_b, kind_a)]
                                {
                                    let slot = &mut table
                                        [((g * groups + h) * 9 + k * 3 + l) * variants + variant];
                                    *slot = (*slot).max(rule.clearance);
                                }
                            }
                        }
                    }
                }
            }
        }
        let width_floor = board
            .nets
            .iter()
            .map(|net| rules.width_floor.get(&net.name).copied().unwrap_or(0.0))
            .collect();
        Self {
            groups,
            net_group,
            no_net_group,
            layer_variant,
            variants,
            variant_layer,
            layer_widths,
            width_floor,
            same_net_hole_to_hole: rules.same_net_hole_to_hole.max(0.0),
            table,
        }
    }

    /// The clearance the rules ask between an item of kind `kind_a` of
    /// group `a` and one of kind `kind_b` of group `b` on layers of
    /// `variant` (0: nothing beyond the classes).
    #[inline]
    pub fn extra(&self, a: usize, kind_a: usize, b: usize, kind_b: usize, variant: usize) -> f64 {
        self.table[((a * self.groups + b) * 9 + kind_a * 3 + kind_b) * self.variants + variant]
    }

    /// `extra` on `layer`.
    pub fn extra_on(&self, a: usize, kind_a: usize, b: usize, kind_b: usize, layer: usize) -> f64 {
        self.extra(a, kind_a, b, kind_b, self.layer_variant[layer])
    }

    /// The largest `extra` over every layer variant (a through via meets
    /// the other item on every layer they share).
    pub fn extra_anywhere(&self, a: usize, kind_a: usize, b: usize, kind_b: usize) -> f64 {
        (0..self.variants)
            .map(|variant| self.extra(a, kind_a, b, kind_b, variant))
            .fold(0.0, f64::max)
    }

    /// The group of an obstacle's net.
    pub fn obstacle_group(&self, obstacle: &Obstacle) -> usize {
        obstacle
            .net
            .map_or(self.no_net_group, |net| self.net_group[net as usize])
    }

    /// The largest clearance any rule asks.
    pub fn largest(&self) -> f64 {
        self.table.iter().copied().fold(0.0, f64::max)
    }

    /// Whether any clearance rule holds anywhere.
    pub fn has_clearances(&self) -> bool {
        self.table.iter().any(|value| *value > 0.0)
    }
}

/// What kind of item a copper obstacle is, for the rules: the lowering's
/// labels tell its existing tracks and vias from pads; copper graphics
/// (`... copper fp_line`, `copper polygon`) are none of them.
pub fn obstacle_kind(obstacle: &Obstacle) -> Option<usize> {
    if obstacle.kind != ObstacleKind::Copper {
        return None;
    }
    match obstacle.label.as_str() {
        "existing track" => Some(TRACK),
        "existing via" => Some(VIA),
        label if label.starts_with("copper ") || label.contains(" copper ") => None,
        _ => Some(PAD),
    }
}

/// The rule classes of a board split by the rules: per class its group
/// and its width floor, the board with the classes split, and per net
/// its class.
#[derive(Clone, Debug)]
pub struct ClassRules {
    pub resolved: Resolved,
    /// Per rule class (after the split, neck classes too).
    pub class_group: Vec<usize>,
    pub class_floor: Vec<f64>,
    /// Per rule class: its track width per layer, where it differs by
    /// layer (`NetRules::layer_widths`; the class's `trace_width` is then
    /// the widest), else empty: the class's width on every layer.
    pub class_widths: Vec<Vec<f64>>,
    /// Per rule class: its width floor per layer (the neck-downs), where it
    /// differs by layer, else empty: `class_floor` on every layer.
    pub class_floors: Vec<Vec<f64>>,
    /// Whether any clearance rule holds (`Resolved::has_clearances`).
    pub clearances: bool,
}

/// What tells two split classes apart beyond their `RuleClass`: the
/// group, the width floor, and the widths and floors per layer.
type ClassKey = (usize, u64, Vec<u64>, Vec<u64>);

fn class_key(group: usize, floor: f64, widths: &[f64], floors: &[f64]) -> ClassKey {
    (
        group,
        floor.to_bits(),
        widths.iter().map(|width| width.to_bits()).collect(),
        floors.iter().map(|floor| floor.to_bits()).collect(),
    )
}

impl ClassRules {
    /// `board` with every net's class split by its group and width floor
    /// (a floor above the class's width raises it) and its widths per
    /// layer, and the pours' brush classes by their net's group. A class
    /// keeps its index for the first nets that need no other (so that
    /// rules which split nothing, as the same-net hole spacing alone,
    /// leave the classes as they are).
    pub fn split(board: &Board, rules: &NetRules) -> (Board, Self) {
        let resolved = Resolved::new(rules, board);
        let mut split = board.clone();
        let mut keys: Vec<Option<ClassKey>> = vec![None; board.classes.len()];
        let mut class_floor = vec![0.0; board.classes.len()];
        let mut class_widths: Vec<Vec<f64>> = vec![Vec::new(); board.classes.len()];
        let mut class_floors: Vec<Vec<f64>> = vec![Vec::new(); board.classes.len()];
        let mut class_of = |original: usize,
                            rules: RuleClass,
                            group: usize,
                            floor: f64,
                            per_layer: (Vec<f64>, Vec<f64>),
                            classes: &mut Vec<RuleClass>| {
            let key = class_key(group, floor, &per_layer.0, &per_layer.1);
            if keys[original].is_none() && classes[original] == rules {
                keys[original] = Some(key);
                class_floor[original] = floor;
                class_widths[original] = per_layer.0;
                class_floors[original] = per_layer.1;
                return original;
            }
            (0..classes.len())
                .find(|&known| classes[known] == rules && keys[known].as_ref() == Some(&key))
                .unwrap_or_else(|| {
                    classes.push(rules);
                    keys.push(Some(key));
                    class_floor.push(floor);
                    class_widths.push(per_layer.0);
                    class_floors.push(per_layer.1);
                    classes.len() - 1
                })
        };
        for (index, net) in board.nets.iter().enumerate() {
            let floor = resolved.width_floor[index];
            let mut rules = board.classes[net.class];
            rules.trace_width = rules.trace_width.max(floor);
            let mut per_layer = (Vec::new(), Vec::new());
            let bounds = &resolved.layer_widths[index];
            if !bounds.is_empty() {
                let widths: Vec<f64> = bounds
                    .iter()
                    .map(|bounds| bounds.width(rules.trace_width).max(floor))
                    .collect();
                let floors: Vec<f64> = bounds.iter().map(|bounds| bounds.min.max(floor)).collect();
                let uniform = |values: &[f64]| values.windows(2).all(|pair| pair[0] == pair[1]);
                if uniform(&widths) && uniform(&floors) {
                    // The same on every layer: one width, one floor.
                    rules.trace_width = widths[0];
                    let floor = floors[0];
                    let class = class_of(
                        net.class,
                        rules,
                        resolved.net_group[index],
                        floor,
                        per_layer,
                        &mut split.classes,
                    );
                    split.nets[index].class = class;
                    continue;
                }
                rules.trace_width = widths.iter().copied().fold(0.0, f64::max);
                per_layer = (widths, floors);
            }
            let class = class_of(
                net.class,
                rules,
                resolved.net_group[index],
                floor,
                per_layer,
                &mut split.classes,
            );
            split.nets[index].class = class;
        }
        for (index, plane) in board.planes.iter().enumerate() {
            let group = resolved.net_group[plane.net as usize];
            let class = class_of(
                plane.class,
                board.classes[plane.class],
                group,
                0.0,
                (Vec::new(), Vec::new()),
                &mut split.classes,
            );
            split.planes[index].class = class;
        }
        let no_net = resolved.no_net_group;
        (
            split,
            Self {
                class_group: keys
                    .into_iter()
                    .map(|key| key.map_or(no_net, |key| key.0))
                    .collect(),
                class_floor,
                class_widths,
                class_floors,
                clearances: resolved.has_clearances(),
                resolved,
            },
        )
    }

    /// The neck width of `class` on `board`: the board's, or the class's
    /// floor where that is wider (on every layer: the largest floor of a
    /// class with floors per layer).
    pub fn neck_width(&self, board: &Board, class: usize) -> f64 {
        board.neck_width.max(
            self.class_floors[class]
                .iter()
                .copied()
                .fold(self.class_floor[class], f64::max),
        )
    }

    /// The track width of `class` on `layer`.
    #[inline]
    pub fn width_on(&self, board: &Board, class: usize, layer: usize) -> f64 {
        self.class_widths[class]
            .get(layer)
            .copied()
            .unwrap_or(board.classes[class].trace_width)
    }

    /// The width floor of `class` on `layer`.
    pub fn floor_on(&self, class: usize, layer: usize) -> f64 {
        self.class_floors[class]
            .get(layer)
            .copied()
            .unwrap_or(self.class_floor[class])
    }

    /// The neck width of `class` on `layer`: the board's, or the class's
    /// floor there where that is wider, and never wider than the class's
    /// track there.
    pub fn neck_width_on(&self, board: &Board, class: usize, layer: usize) -> f64 {
        let neck = board.neck_width.max(self.floor_on(class, layer));
        if self.class_widths[class].is_empty() {
            neck
        } else {
            neck.min(self.width_on(board, class, layer))
        }
    }

    /// Whether some class has widths per layer.
    pub fn has_layer_widths(&self) -> bool {
        self.class_widths.iter().any(|widths| !widths.is_empty())
    }

    /// The clearance the rules ask between an item of kind `kind` of
    /// `class` and copper `obstacle` on `layer` (0: none).
    pub fn obstacle_extra(
        &self,
        class: usize,
        kind: usize,
        obstacle: &Obstacle,
        layer: usize,
    ) -> f64 {
        if !self.clearances {
            return 0.0;
        }
        let (a, b) = (
            self.class_group[class],
            self.resolved.obstacle_group(obstacle),
        );
        match obstacle_kind(obstacle) {
            // A pad's net leaves it by a stub off the lattice, a track that
            // only the pad's clearance keeps from other nets' copper: the
            // pad keeps what tracks of its net keep too (tuxkeychain's
            // 0.127 mm between outer-layer tracks over classes of 0.1:
            // stubs 0.104 mm from other nets' tracks next to their pads).
            Some(PAD) => self
                .resolved
                .extra_on(a, kind, b, PAD, layer)
                .max(self.resolved.extra_on(a, kind, b, TRACK, layer)),
            Some(other) => self.resolved.extra_on(a, kind, b, other, layer),
            None => 0.0,
        }
    }

    /// `obstacle_extra` for a via, on every layer of the obstacle.
    pub fn obstacle_extra_via(&self, class: usize, obstacle: &Obstacle, layers: usize) -> f64 {
        (0..layers)
            .filter(|layer| obstacle.layers & (1 << layer) != 0)
            .map(|layer| self.obstacle_extra(class, VIA, obstacle, layer))
            .fold(0.0, f64::max)
    }
}

// Copyright (C) 2026 Toit contributors.

//! The lowering's two halves of state: what it reads (`Context`: the board,
//! its rules and the facts derived from them before any item is lowered)
//! and what it assembles (`Builder`: the router's board in the making).

use super::*;
use pcb_router as core;
use std::collections::{HashMap, HashSet};

/// The solder mask rules the pads' apertures and the board's openings are
/// lowered by.
pub(super) struct MaskRules {
    /// The board's mask expansion. A pad's solder mask aperture (the pad
    /// grown by its mask expansion) bares the copper around it: another
    /// net's copper near it bridges. Only reaches past the copper
    /// clearance matter. The board's mask expansion is
    /// `pad_to_mask_clearance` (KiCad's setup has no `solder_mask_margin`;
    /// the rule below read that key until 2026-10-15, so only pads and
    /// footprints with their own margin had an aperture).
    pub board_pad_to_mask: f64,
    /// The board's `solder_mask_min_width`, 0 for none.
    pub min_mask_web: f64,
    /// Half the minimum web and a little more, 0 without a web.
    pub pad_mask_reach: f64,
    /// The project's solder-mask-to-copper clearance.
    pub mask_to_copper: f64,
    /// Whether the board's vias are tented, front and back.
    pub tented: [bool; 2],
    /// KiCad's exact rule where its bridges are errors; the rule of before
    /// where the designer rated them lower.
    pub strict: bool,
}

/// What the lowering reads and never changes: the board, its rules, and
/// what is derived from them before the first item is lowered.
pub(super) struct Context<'a> {
    pub pcb: &'a Expr,
    pub config: &'a KiCadBoardRouterConfig,
    /// The planes' `connect`.
    pub connect_pours: bool,
    pub net_names: crate::kicad_nets::NetNames,
    pub board_version: Option<u64>,
    /// KiCad 9 and before name the net of a track, via or graphic by code.
    pub net_codes: BTreeMap<String, String>,
    pub loops: outline::BoardLoops,
    pub layers: LayerTable,
    /// Prepared fan-outs (`bga_fanout`): every footprint's SMD pads are
    /// kept for the plan once the board's copper is lowered.
    pub fanout_on: bool,
    /// Vias the via-in-pad rule puts in enclosed pads (a fine-pitch BGA's
    /// inner balls), by the pad's label: each copper of the pad's net on
    /// every layer and the pad's terminal there (`via_in_pad`).
    /// (A panel repeats references: a via is its pad's by place too.)
    pub pad_vias: BTreeMap<String, Vec<PlannedVia>>,
    /// With `use_narrow_signals`, signal nets of the Default class (not
    /// poured, not named as a ground or a rail; `narrows_class_of`) route at
    /// the project's smallest predefined width: the poured nets.
    pub poured: HashSet<String>,
    /// Whether differential pairs keep to their layers (`pair_layers`).
    pub pairs_on: bool,
    pub mask: MaskRules,
    /// The smallest clearance of every class.
    pub smallest_clearance: f64,
    /// Copper of no net (a logo, a footprint's graphic) is in KiCad's
    /// Default class: other copper keeps the larger of both clearances
    /// (tuxkeychain's GRAPHIC3).
    pub no_net_clearance: f64,
}

impl<'a> Context<'a> {
    /// Reads `pcb`'s net names, layers, outline, planned vias in pads and
    /// mask rules under `config`.
    pub fn new(
        pcb: &'a Expr,
        config: &'a KiCadBoardRouterConfig,
        connect_pours: bool,
    ) -> Result<Self, String> {
        // The board's net facts; noted on this thread too, for the writer
        // and the project rules that read them after (`note_net_names`).
        let net_names = crate::note_net_names(pcb);
        let board_version = form_atom(pcb, "version", 1).and_then(|version| version.parse().ok());
        let net_codes = crate::net_codes(pcb);
        let loops = outline::board_loops(pcb)?;
        let layers = LayerTable::from_pcb(pcb)?;
        let fanout_on = bga_fanout::bga_fanout_on(config);
        let mut pad_vias: BTreeMap<String, Vec<PlannedVia>> = BTreeMap::new();
        for via in plan_vias_in_pads(pcb, config).vias {
            pad_vias.entry(via.label.clone()).or_default().push(via);
        }
        let poured: HashSet<String> = if config.use_narrow_signals == Some(true) {
            pours(pcb, &layers)?
                .into_iter()
                .map(|pour| pour.net)
                .collect()
        } else {
            Default::default()
        };
        let pairs_on = config.diff_pairs_on() && config.engine.as_deref() != Some("topological");
        let board_pad_to_mask = pcb
            .child("setup")
            .and_then(|setup| form_f64(setup, "pad_to_mask_clearance", 1).ok())
            .unwrap_or(0.0);
        let min_mask_web = pcb
            .child("setup")
            .and_then(|setup| form_f64(setup, "solder_mask_min_width", 1).ok())
            .unwrap_or(0.0)
            .max(0.0);
        let pad_mask_reach = if min_mask_web > 0.0 {
            min_mask_web / 2.0 + 0.01
        } else {
            0.0
        };
        let mask_to_copper = config.solder_mask_to_copper_mm.unwrap_or(0.0).max(0.0);
        let tented = vias_tented(pcb);
        let strict = !matches!(
            config.solder_mask_bridge_severity.as_deref(),
            Some("warning" | "ignore")
        );
        let smallest_clearance = config
            .connection_rules
            .values()
            .chain(config.default_rules.iter())
            .map(|rules| rules.clearance_mm)
            .fold(f64::INFINITY, f64::min);
        let no_net_clearance = config
            .default_rules
            .as_ref()
            .map_or(0.0, |rules| rules.clearance_mm);
        Ok(Self {
            pcb,
            config,
            connect_pours,
            net_names,
            board_version,
            net_codes,
            loops,
            layers,
            fanout_on,
            pad_vias,
            poured,
            pairs_on,
            mask: MaskRules {
                board_pad_to_mask,
                min_mask_web,
                pad_mask_reach,
                mask_to_copper,
                tented,
                strict,
            },
            smallest_clearance,
            no_net_clearance,
        })
    }

    /// The layers a differential pair's net `name` routing at `width` keeps
    /// to, 0 for all. A pair's nets (`diff_pairs`) keep off the layers the
    /// custom rules disallow their tracks on (`disallow track`: ducktop2's
    /// ground and impedance layers), and off those where a custom rule's
    /// track width bounds do not hold the width they route at (ducktop2's
    /// DIFF_90: 0.1796 mm outside, at most 0.111 on In2.Cu; a class has one
    /// width), as the pair's coupled runs do; the stubs and the fallback
    /// route them in their class.
    pub fn pair_layers(&self, name: &str, width: f64) -> core::LayerMask {
        let Some(rule) = self
            .config
            .diff_pair_rules
            .get(name)
            .filter(|_| self.pairs_on)
        else {
            return 0;
        };
        let count = rule.disallowed.len().min(32);
        let other_width = |layer: usize| {
            rule.width_bounds
                .get(layer)
                .is_some_and(|(min, max)| width < min - 1.0e-6 || width > max + 1.0e-6)
        };
        let allowed = (0..count)
            .filter(|layer| !rule.disallowed[*layer] && !other_width(*layer))
            .fold(0, |mask: core::LayerMask, layer| mask | (1 << layer));
        if allowed.count_ones() as usize == count {
            0
        } else {
            allowed
        }
    }

    /// The layers of the class of net `name` routing at `width`, 0 for all.
    /// Every net keeps off the layers a custom rule disallows its tracks on
    /// (`track_disallowed`: ducktop2's solid ground layers for every net
    /// but GND), on every rung: the class's layers, whatever the rung does
    /// with the pours (until 2026-10-27 only the exclusive planes kept
    /// other nets off them, and the rungs without those routed over the
    /// ground layers). A net the rules close on every layer keeps every
    /// layer.
    pub fn class_layers(&self, name: &str, width: f64) -> core::LayerMask {
        let all_layers = self.layers.all();
        let pair = match self.pair_layers(name, width) {
            0 => all_layers,
            mask => mask,
        };
        let open = self
            .config
            .track_disallowed
            .get(name)
            .map_or(all_layers, |closed| all_layers & !closed);
        match pair & open {
            0 => self.pair_layers(name, width),
            mask if mask == all_layers => 0,
            mask => mask,
        }
    }
}

/// The router's board in the making: what the stages add, in the order
/// they add it.
#[derive(Default)]
pub(super) struct Builder {
    pub classes: Vec<core::RuleClass>,
    pub nets: Vec<core::Net>,
    pub net_ids: BTreeMap<String, core::NetId>,
    pub obstacles: Vec<core::Obstacle>,
    /// Rule areas that keep zone fills out: (layers, polygon).
    pub pour_keepouts: Vec<PourKeepout>,
    pub solid_pads: Vec<String>,
    pub isolated_pads: Vec<String>,
    /// Regions where a pour stays whole (`keep_whole`): their rule areas'
    /// pour net, layers and polygon, lowered once the nets are known.
    pub keep_whole: Vec<KeepWholeRegion>,
    /// Rule areas the fill passes (`core::Board::fill_passes`).
    pub fill_passes: Vec<usize>,
    /// Openings in the solder mask (graphics on a mask layer), placed once
    /// the pads under them are known.
    pub mask_openings: Vec<MaskOpening>,
    /// Board copper graphics, with the net they carry, placed once the nets
    /// of the pads are known.
    pub copper_graphics: Vec<CopperGraphic>,
    /// `solid_pads` and `isolated_pads` per pad, by obstacle index.
    pub solid_pad_obstacles: Vec<usize>,
    pub isolated_pad_obstacles: Vec<usize>,
    /// Per pad (by obstacle index), its footprint's jumper group: KiCad 9's
    /// `(duplicate_pad_numbers_are_jumpers yes)` joins the pads of one
    /// number, `(jumper_pad_groups ("1" "2") ...)` the pads of the numbers
    /// of a group, without copper (kicad-cli 10.0.7: no unconnected item
    /// between them).
    pub jumper_groups: HashMap<usize, (usize, String)>,
    /// The footprints lowered so far.
    pub footprint_ordinal: usize,
    /// A padstack pad's copper on the layers its stack names, by the pad's
    /// own obstacle: merged into the pad's terminal (`MergedPad::padstack`).
    pub padstack_copper: Vec<(usize, usize)>,
    /// SMD pads no via may overlap (`core::Board::no_via_pads`): every SMD
    /// pad's copper but an exposed pad's, unless the board's vias are
    /// filled and capped (`via_in_pad`).
    pub no_via_pads: Vec<usize>,
    /// With prepared fan-outs (`bga_fanout`): every footprint's SMD pads,
    /// for the plan once the board's copper is lowered.
    pub fanout_parts: Vec<bga_fanout::FanoutPart>,
    /// The pads holding a via already (`Context::pad_vias`).
    pub holding_vias: BTreeSet<usize>,
    /// The gaps between adjacent SMD pads of a footprint's pin rows
    /// (`core::Board::pin_row_gaps`, `pin_row_gaps`).
    pub pin_row_gaps: Vec<core::PinRowGap>,
    pub pad_spokes: HashMap<usize, core::PadSpokes>,
    pub merged_pads: Vec<core::MergedPad>,
}

impl Builder {
    /// The id of the net `name`, added with its class on first sight.
    pub fn net_id(&mut self, ctx: &Context, name: &str) -> Result<core::NetId, String> {
        if let Some(id) = self.net_ids.get(name) {
            return Ok(*id);
        }
        let config = ctx.config;
        let rules = config
            .connection_rules
            .get(name)
            .or(config.default_rules.as_ref())
            .ok_or_else(|| format!("no routing rules for connection {name:?}"))?;
        let narrow = match (config.use_narrow_signals, config.narrow_signal_mm) {
            (Some(true), Some(width))
                if width < rules.trace_width_mm
                    && !config.ruled_widths.contains(name)
                    && config.narrows_class_of(name, rules)
                    && !ctx.poured.contains(name)
                    && !crate::quality::ground(name)
                    && !crate::quality::rail_name(name) =>
            {
                Some(width)
            }
            _ => None,
        };
        let class = core::RuleClass {
            trace_width: narrow
                .unwrap_or(rules.trace_width_mm)
                .max(config.width_floor()),
            clearance: rules.clearance_mm,
            via_diameter: rules.via_size_mm,
            via_drill: rules.via_drill_mm,
            layers: ctx.class_layers(name, rules.trace_width_mm),
        };
        let class_index = self
            .classes
            .iter()
            .position(|existing| *existing == class)
            .unwrap_or_else(|| {
                self.classes.push(class);
                self.classes.len() - 1
            });
        let id = self.nets.len() as core::NetId;
        self.nets.push(core::Net {
            name: name.to_string(),
            class: class_index,
            terminals: Vec::new(),
        });
        self.net_ids.insert(name.to_string(), id);
        Ok(id)
    }

    /// The routed net of a board item (a track, a via), by id: None for an
    /// item of no net or of a net the router does not route.
    pub fn item_net_id(
        &mut self,
        ctx: &Context,
        item: &Expr,
    ) -> Result<Option<core::NetId>, String> {
        item_net(item, &ctx.net_codes)
            .filter(|raw| ctx.net_names.routable(raw))
            .map(|raw| ctx.net_names.normalize(raw))
            .map(|name| self.net_id(ctx, name))
            .transpose()
    }

    /// The terminal (its index and anchor) that pad obstacle `pad` of `net`
    /// is, or was merged into.
    pub fn terminal_host(&self, net: core::NetId, pad: usize) -> Option<(usize, [f64; 2])> {
        let terminals = &self.nets[net as usize].terminals;
        terminals
            .iter()
            .position(|terminal| terminal.pad == pad)
            .map(|host| (host, terminals[host].anchor))
            .or_else(|| {
                self.merged_pads
                    .iter()
                    .find(|merged| merged.net == net && merged.pad == pad)
                    .map(|merged| (merged.host, merged.anchor))
            })
    }
}

/// The id of the net `name` among `nets`, if it has one.
pub(super) fn net_ids_lookup(nets: &[core::Net], name: &str) -> Option<core::NetId> {
    nets.iter()
        .position(|net| net.name == name)
        .map(|index| index as core::NetId)
}

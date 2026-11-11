// Copyright (C) 2026 Toit contributors.

//! The board's copper layers and the layer names of its items.

use super::*;
use pcb_router as core;

/// The board's copper layers in stack order: front, inner 1..n, back.
pub struct LayerTable {
    pub names: Vec<String>,
}

impl LayerTable {
    pub fn from_pcb(pcb: &Expr) -> Result<Self, String> {
        let mut names: Vec<String> = pcb
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .filter_map(|entry| entry.children().get(1).and_then(Expr::atom))
            .filter(|name| name.ends_with(".Cu"))
            .map(str::to_owned)
            .collect();
        let rank = |name: &str| -> (u8, u32) {
            match name {
                "F.Cu" => (0, 0),
                "B.Cu" => (2, 0),
                _ => (
                    1,
                    name.trim_start_matches("In")
                        .trim_end_matches(".Cu")
                        .parse()
                        .unwrap_or(u32::MAX),
                ),
            }
        };
        names.sort_by_key(|name| rank(name));
        if names.first().map(String::as_str) != Some("F.Cu")
            || names.last().map(String::as_str) != Some("B.Cu")
            || names.len() > 32
        {
            return Err(format!(
                "unsupported copper layer stack {names:?}: expected F.Cu, inner layers, B.Cu"
            ));
        }
        Ok(Self { names })
    }

    // Never empty: `new` requires F.Cu and B.Cu.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|layer| layer == name)
    }

    /// Every copper layer: up to 32, a bit each (`1 << 32` would wrap
    /// to none in a release build).
    pub fn all(&self) -> core::LayerMask {
        if self.len() >= 32 {
            core::LayerMask::MAX
        } else {
            (1 << self.len()) - 1
        }
    }

    /// Mask for a KiCad layer list such as `("F.Cu" "In1.Cu")`, `*.Cu`
    /// (all copper) or `F&B.Cu` (both outer layers).
    pub fn mask_of<'a>(
        &self,
        names: impl Iterator<Item = &'a str>,
    ) -> Result<core::LayerMask, String> {
        let mut mask = 0;
        for name in names {
            match name {
                "*.Cu" => mask |= self.all(),
                "F&B.Cu" => mask |= 1 | 1 << (self.len() - 1),
                // A copper layer the stackup does not have (a footprint
                // made for more layers) carries nothing, as in KiCad.
                name if name.ends_with(".Cu") => {
                    if let Some(index) = self.index(name) {
                        mask |= 1 << index;
                    }
                }
                _ => {}
            }
        }
        Ok(mask)
    }

    /// The copper layers of a pad. A plated through-hole pad is copper
    /// on every layer to KiCad 10.0.7 whatever its `layers` list names:
    /// pcbnew loads koeg's `(layers "*.Mask" "F.Cu" "B.Cu" "In1.Cu")`
    /// pin headers, `("F.Cu" "*.Mask")` and `("In1.Cu")` on all four
    /// copper layers, and kicad-cli's DRC measures In2.Cu tracks against
    /// the pad's ring there (koeg's J5.3: clearance 0.01 mm, a short).
    /// Any other pad is copper on the layers it lists.
    pub fn pad_mask(&self, pad: &Expr) -> Result<core::LayerMask, String> {
        if pad.children().get(2).and_then(Expr::atom) == Some("thru_hole") {
            return Ok(self.all());
        }
        self.mask_of_item(pad)
    }

    /// Mask of an item with either a `layer` or a `layers` form.
    pub fn mask_of_item(&self, item: &Expr) -> Result<core::LayerMask, String> {
        if let Some(layer) = form_atom(item, "layer", 1) {
            return self.mask_of(std::iter::once(layer));
        }
        let names: Vec<&str> = item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .collect();
        self.mask_of(names.into_iter())
    }
}

/// The copper layers of a via: a through via's every layer, a blind,
/// buried or micro via's (`(via blind ... (layers "F.Cu" "In1.Cu"))`)
/// the layers from its first to its last. The router's own vias are
/// through vias; another via is copper on its own layers only.
pub(super) fn via_layers(via: &Expr, layers: &LayerTable) -> core::LayerMask {
    let ends: Vec<usize> = via
        .child("layers")
        .map(Expr::children)
        .unwrap_or_default()
        .iter()
        .skip(1)
        .filter_map(Expr::atom)
        .filter_map(|name| layers.index(name.trim_matches('"')))
        .collect();
    match ends.as_slice() {
        [first, last] => {
            let (low, high) = ((*first).min(*last), (*first).max(*last));
            (low..=high).fold(0, |mask, layer| mask | 1 << layer)
        }
        _ => layers.all(),
    }
}

/// The layer names of an item with either a `layer` or a `layers` form.
pub(super) fn item_layer_names(item: &Expr) -> Vec<&str> {
    match form_atom(item, "layer", 1) {
        Some(name) => vec![name],
        None => item
            .child("layers")
            .map(Expr::children)
            .unwrap_or_default()
            .iter()
            .skip(1)
            .filter_map(Expr::atom)
            .collect(),
    }
}

/// The outer copper layers whose solder mask `names` open (F.Mask: the
/// front, B.Mask: the back).
pub(super) fn mask_sides(names: &[&str], layer_count: usize) -> Vec<usize> {
    names
        .iter()
        .filter_map(|name| match *name {
            "F.Mask" => Some(0),
            "B.Mask" => Some(layer_count - 1),
            _ => None,
        })
        .collect()
}

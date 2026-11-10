// Copyright (C) 2026 Toit contributors.

//! Openings in the solder mask (mask graphics, texts, mask-only pads):
//! queued while the board's items are lowered, placed as keepouts of the
//! net they expose once every pad and pour is known.

use super::*;
use pcb_router as core;

/// An opening in the solder mask, placed once the pads under it are known.
pub(super) struct MaskOpening {
    pub shapes: Vec<core::Shape>,
    /// The outer copper layer it bares.
    pub layer: usize,
    pub kind: String,
    /// How far the opening reaches past its shapes (a mask-only pad's mask
    /// expansion).
    pub margin: f64,
    /// The aperture's mask expansion as KiCad's bridge test takes it (a
    /// graphic's own `solder_mask_margin`, else the board's
    /// `pad_to_mask_clearance`).
    pub expansion: f64,
}

impl Builder {
    /// The queued mask openings, each a keepout of the one net it exposes
    /// or of none. A mask opening exposes the copper under it: pads, or
    /// else the fill of a pour. Over one net (a pad or a pour exposed on
    /// purpose) it is that net's: it may route there, others would bridge
    /// to it. Over several (J_LCD1's opening over its eight pads in Sisu)
    /// or none, nothing new goes under it; pads inside leave it by their
    /// stubs. The net is the whole graphic's.
    pub(super) fn place_mask_openings(&mut self, ctx: &Context, pours: &[Pour]) {
        // KiCad's bridge test drops mask slivers under the minimum web
        // width: copper of another net within half of it of an opening is
        // exposed (link: a track 0.025 mm from an F.Mask polygon bridged it
        // to GND with a 0.1 mm minimum; at 0.07 mm it did not).
        let mask_reach = ctx
            .pcb
            .child("setup")
            .and_then(|setup| form_f64(setup, "solder_mask_min_width", 1).ok())
            .map_or(0.0, |width| width / 2.0 + 0.01);
        // KiCad's bridge test (DRC_TEST_PROVIDER_SOLDER_MASK, KiCad 10)
        // exposes copper within the project's solder-mask-to-copper
        // clearance plus the aperture's expansion, and never less than the
        // web rule above.
        for opening in std::mem::take(&mut self.mask_openings) {
            let MaskOpening {
                shapes,
                layer,
                kind,
                margin,
                expansion,
            } = opening;
            let reach = (mask_reach + margin).max(ctx.mask.mask_to_copper + expansion);
            let Some(first) = shapes.first() else {
                continue;
            };
            let bounds = shapes
                .iter()
                .skip(1)
                .fold(first.aabb(), |bounds, shape| bounds.union(shape.aabb()))
                .inflated(margin);
            let mut exposed: BTreeSet<core::NetId> = self
                .obstacles
                .iter()
                .filter(|obstacle| {
                    obstacle.kind == core::ObstacleKind::Copper
                        && obstacle.layers & (1 << layer) != 0
                        && obstacle.shape.aabb().intersects(bounds)
                })
                .filter_map(|obstacle| obstacle.net)
                .collect();
            let center = [
                (bounds.minimum[0] + bounds.maximum[0]) / 2.0,
                (bounds.minimum[1] + bounds.maximum[1]) / 2.0,
            ];
            let under: Vec<&Pour> = pours
                .iter()
                .filter(|pour| {
                    pour.layers & (1 << layer) != 0
                        && core::geometry::point_in_polygon(center, &pour.polygon)
                })
                .collect();
            // A pour's fill keeps its clearance from pads: it shows through
            // only where the opening has no pad of its own.
            if exposed.is_empty()
                && let Some(top) = under.iter().map(|pour| pour.priority).max()
            {
                exposed.extend(
                    under
                        .iter()
                        .filter(|pour| pour.priority == top)
                        .filter_map(|pour| net_ids_lookup(&self.nets, &pour.net)),
                );
            }
            if std::env::var_os("PCB_ROUTER_DEBUG").is_some() {
                eprintln!(
                    "mask opening ({kind}) on layer {layer}: {:?}, nets {exposed:?}",
                    bounds
                );
            }
            // Over bare board an opening is no net's: copper of two nets
            // under one text bridges (jetson-nano's B.Mask title over three
            // nets' vias and tracks).
            let net = if exposed.len() == 1 {
                exposed.first().copied()
            } else {
                None
            };
            self.obstacles
                .extend(shapes.into_iter().map(|shape| core::Obstacle {
                    shape,
                    layers: 1 << layer,
                    kind: core::ObstacleKind::Keepout,
                    net,
                    clearance: reach,
                    clearance_override: None,
                    plated: false,
                    blocks_tracks: true,
                    blocks_vias: true,
                    label: format!("solder mask opening ({kind})"),
                }));
        }
    }
}

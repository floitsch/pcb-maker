// Copyright (C) 2026 Florian Loitsch.

//! Native evidence for custom pad shapes that fail on their own geometry.

use super::{DrcFinding, is_pad_item, pad_footprint};
use crate::kicad_nets::{footprint_reference, footprint_uuid, item_net, net_codes};
use crate::{Expr, parse};
use serde::Serialize;
use std::fs;
use std::path::Path;

pub(super) const KIND: &str = "padstack_invalid";
const REASON: &str = "custom pad shape must resolve to a single polygon";
const MAX_EXAMPLES: usize = 8;

/// A native KiCad error proven to refer to one custom pad on the saved PCB.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KiCadInvalidPadstack {
    pub reference: String,
    pub pad: String,
    pub net: Option<String>,
    pub layer: Option<String>,
    pub uuid: String,
    pub footprint_id: String,
    pub reason: &'static str,
}

#[derive(Default)]
pub(super) struct Review {
    pub total: usize,
    pub examples: Vec<KiCadInvalidPadstack>,
}

impl Review {
    pub fn assess(findings: &[&DrcFinding], saved_board: Option<&Path>) -> Self {
        let mut review = Self::default();
        let candidates = findings.iter().filter(|finding| {
            finding.kind == KIND
                && finding.description
                    == "Padstack is not valid (custom pad shape must resolve to a single polygon)"
        });
        if candidates.clone().next().is_none() {
            return review;
        }
        let Some(board) = saved_board
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| parse(&text).ok())
        else {
            return review;
        };
        let codes = net_codes(&board);
        for finding in candidates {
            let [item] = &finding.items[..] else { continue };
            let [label] = &finding.item_descriptions[..] else {
                continue;
            };
            let Some(uuid) = item.uuid.as_deref().filter(|uuid| !uuid.is_empty()) else {
                continue;
            };
            let Some(reference) = pad_footprint(label).filter(|_| is_pad_item(label)) else {
                continue;
            };
            let mut matches = board
                .children()
                .iter()
                .filter(|part| matches!(part.head(), Some("footprint" | "module")))
                .flat_map(|part| {
                    part.children()
                        .iter()
                        .filter(move |pad| pad.head() == Some("pad"))
                        .map(move |pad| (part, pad))
                })
                .filter(|(_, pad)| {
                    footprint_uuid(pad).is_some_and(|saved| saved.eq_ignore_ascii_case(uuid))
                });
            let Some((part, pad)) = matches.next() else {
                continue;
            };
            if matches.next().is_some()
                || footprint_reference(part).as_deref() != Some(reference)
                || pad.children().get(3).and_then(Expr::atom) != Some("custom")
            {
                continue;
            }
            let Some(footprint_id) = part.children().get(1).and_then(Expr::atom) else {
                continue;
            };
            let Some(number) = pad.children().get(1).and_then(Expr::atom) else {
                continue;
            };
            let net = item_net(pad, &codes)
                .filter(|net| !net.is_empty())
                .map(crate::outline_pieces::unescaped);
            let Some((named_pad, owner)) = label.rsplit_once("] of ") else {
                continue;
            };
            let number_and_net = format!("{number} [{}", net.as_deref().unwrap_or("<no net>"));
            if !named_pad
                .split_once(' ')
                .map(|(_, tail)| tail.strip_prefix("pad ").unwrap_or(tail))
                .is_some_and(|tail| tail == number_and_net)
            {
                continue;
            }
            review.total += 1;
            if review.examples.len() < MAX_EXAMPLES {
                review.examples.push(KiCadInvalidPadstack {
                    reference: reference.to_string(),
                    pad: number.to_string(),
                    net,
                    layer: owner.split_once(" on ").map(|(_, layer)| layer.to_string()),
                    uuid: uuid.to_string(),
                    footprint_id: footprint_id.to_string(),
                    reason: REASON,
                });
            }
        }
        review
    }

    pub fn action(&self) -> Option<String> {
        if self.total == 0 {
            return None;
        }
        let pads = self.examples.iter().map(|pad| {
            format!(
                "{}:{} [{}] in `{}`",
                pad.reference,
                pad.pad,
                pad.net.as_deref().unwrap_or("no net"),
                pad.footprint_id
            )
        });
        let mut samples = pads.collect::<Vec<_>>().join(", ");
        if self.total > self.examples.len() {
            samples.push_str(&format!(" and {} more", self.total - self.examples.len()));
        }
        Some(format!(
            "KiCad requires each custom pad to resolve to a single polygon. Affected pads include {samples}. Edit the source footprint library so the copper primitives and pad anchor form one continuous polygon on each affected layer. Keep the pad numbers and intended electrical contacts, update the footprint from the library, and rerun KiCad verification."
        ))
    }
}

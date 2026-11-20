// Copyright (C) 2026 Florian Loitsch.

//! Explicit part ownership on one physical panel piece.

use super::*;
use crate::exit::input_error as error;
use crate::placement_constraints::KiCadConstraintStatus;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadSubBoardConstraint {
    /// Absolute point inside the intended piece.
    pub piece_at_mm: [f64; 2],
    /// Exact footprint references.
    pub parts: Vec<String>,
}

pub(crate) struct Confirmed {
    pub(super) pieces: Arc<Pieces>,
    pub(super) parts: BTreeMap<usize, usize>,
}

impl Confirmed {
    pub(crate) fn read(
        pcb: &Expr,
        problem: &pcb_placer::Problem,
        references: &[String],
        entries: &[KiCadSubBoardConstraint],
    ) -> Result<Option<Self>, String> {
        if entries.is_empty() {
            return Ok(None);
        }
        let loops = outline::board_loops(pcb)?;
        let pieces =
            Pieces::of_loops_cached(&loops.piece_loops, &loops.cutouts, &Cuts::of_board(pcb))
                .ok_or_else(|| error("sub_board needs an outline with more than one piece"))?;
        let mut parts = BTreeMap::new();
        for entry in entries {
            let [x, y] = entry.piece_at_mm;
            let piece = pieces.piece_at([x, y]).ok_or_else(|| {
                error(format!(
                    "sub_board point [{x}, {y}] is off the pieces or on a tab"
                ))
            })?;
            if entry.parts.is_empty() {
                return Err(error(
                    "sub_board requires at least one exact part reference",
                ));
            }
            for reference in &entry.parts {
                let mut matches = references
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| *name == reference);
                let Some((part, _)) = matches.next() else {
                    return Err(error(format!("unknown sub_board part {reference}")));
                };
                if matches.next().is_some() || problem.components[part].pins.is_empty() {
                    return Err(error(format!(
                        "sub_board part {reference} must be a unique footprint with electrical pins"
                    )));
                }
                if let Some(prior) = parts.insert(part, piece)
                    && prior != piece
                {
                    return Err(error(format!(
                        "conflicting sub_board requests for {reference}"
                    )));
                }
                let component = &problem.components[part];
                if component.fixed
                    && !pin_centers_on(&pieces, piece, component, problem.poses[part])
                {
                    return Err(error(format!(
                        "sub_board requests piece {} for held part {reference}, whose pins are elsewhere",
                        piece + 1
                    )));
                }
            }
        }
        Ok(Some(Self { pieces, parts }))
    }

    pub(crate) fn report(
        &self,
        problem: &pcb_placer::Problem,
        poses: &[pcb_placer::Pose],
        references: &[String],
    ) -> Vec<KiCadConstraintStatus> {
        self.parts
            .iter()
            .map(|(&part, &piece)| {
                let component = &problem.components[part];
                let pose = poses[part];
                let counts = component
                    .pins
                    .iter()
                    .filter(|pin| {
                        self.pieces.piece_at(component.pin_position(pin, pose)) != Some(piece)
                    })
                    .count();
                KiCadConstraintStatus {
                    kind: "sub_board".into(),
                    part: references[part].clone(),
                    other: Some(format!("piece {}", piece + 1)),
                    satisfied: counts == 0,
                    violation_mm: 0.0,
                    distance_mm: None,
                    pad_near: None,
                    body_near_pin: None,
                    near_geometry: None,
                    detail: Some(format!(
                        "{counts} electrical pin centers outside piece {}",
                        piece + 1
                    )),
                }
            })
            .collect()
    }
}

pub(super) fn pin_centers_on(
    pieces: &Pieces,
    piece: usize,
    component: &pcb_placer::Component,
    pose: pcb_placer::Pose,
) -> bool {
    component
        .pins
        .iter()
        .all(|pin| pieces.piece_at(component.pin_position(pin, pose)) == Some(piece))
}

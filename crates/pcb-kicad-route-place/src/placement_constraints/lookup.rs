// Copyright (C) 2026 Toit contributors.

//! The board's parts as the constraints name them (by reference, in turn
//! or by uuid where several footprints share one, by glob), with the
//! lowered placement's per-footprint tables the stages read.

use super::*;

/// The lowered placement's per-footprint tables and the rows' followers:
/// what every stage of `apply_constraints` looks parts and pads up in.
pub(super) struct Parts<'a> {
    pub(super) references: &'a [String],
    /// How many of the problem's components are the board's footprints
    /// (the first ones).
    pub(super) footprints: usize,
    /// Per footprint, its pad names' local offsets.
    pub(super) pads: &'a [BTreeMap<String, [f64; 2]>],
    pub(super) pad_boxes: &'a [Vec<[f64; 4]>],
    /// Per footprint, the local box of its own keepout zone (an antenna's).
    pub(super) keepouts: &'a [Option<[f64; 4]>],
    /// Per footprint, the local direction it opens towards.
    pub(super) mouths: &'a [Option<[f64; 2]>],
    pub(super) locked: &'a [bool],
    pub(super) uuids: &'a [Option<String>],
    /// Each row follower's leader (`apply_rows`).
    pub(super) leaders: BTreeMap<usize, usize>,
    /// Each row follower's place in its leader's frame.
    pub(super) places: BTreeMap<usize, RowPlace>,
}

impl Parts<'_> {
    /// The part `reference` names (its first footprint); an error for an
    /// unknown reference or a part its row carries.
    pub(super) fn find(&self, reference: &str) -> Result<usize, String> {
        let index = self.references[..self.footprints]
            .iter()
            .position(|candidate| candidate == reference)
            .ok_or_else(|| format!("constraint names unknown part {reference:?}"))?;
        self.not_carried(reference, index)
    }

    /// `index`, unless a row carries it: then the error naming the row's
    /// first part.
    fn not_carried(&self, reference: &str, index: usize) -> Result<usize, String> {
        match self.leaders.get(&index) {
            Some(leader) => Err(format!(
                "{reference} is carried by its row: constrain the row's first part, {}",
                self.references[*leader]
            )),
            None => Ok(index),
        }
    }

    /// The footprint an entry of one kind names, `used` counting that
    /// kind's entries per reference.
    ///
    /// A reference several parts share (a panel's copies): an entry with a
    /// `uuid` is about the footprint with that uuid (from_pcbench writes
    /// one for every entry of a repeated reference). Without one, they are
    /// named in turn: the n-th such entry of one kind for it is about its
    /// n-th part in board order (hand-written constraints). All of them on
    /// its first part asked one connector to touch the left, top and bottom
    /// edges at once (the molecule panels' J3 to J6); in turn, the entries
    /// of a task that writes them only for the copies at an edge bound
    /// other copies (the molecules' fixed J1, J3 and J4 missed 17 edges by
    /// whole tile pitches).
    pub(super) fn instance(
        &self,
        reference: &str,
        uuid: Option<&str>,
        used: &mut BTreeMap<String, usize>,
    ) -> Result<usize, String> {
        let parts: Vec<usize> = (0..self.footprints)
            .filter(|index| self.references[*index] == reference)
            .collect();
        let index = if let Some(uuid) = uuid {
            if parts.is_empty() {
                return self.find(reference);
            }
            *parts
                .iter()
                .find(|index| {
                    self.uuids
                        .get(**index)
                        .and_then(Option::as_deref)
                        .is_some_and(|own| own.eq_ignore_ascii_case(uuid))
                })
                .ok_or_else(|| {
                    format!(
                        "constraint names {reference} with uuid {uuid:?}, which no footprint {reference} has"
                    )
                })?
        } else {
            if parts.len() < 2 {
                return self.find(reference);
            }
            let count = used.entry(reference.to_string()).or_default();
            let index = parts[*count % parts.len()];
            *count += 1;
            index
        };
        self.not_carried(reference, index)
    }

    /// The parts a glob matches. Globs skip the parts rows carry (their
    /// first part stands for them).
    pub(super) fn matching(&self, pattern: &str) -> Vec<usize> {
        (0..self.footprints)
            .filter(|index| {
                !self.leaders.contains_key(index) && glob_matches(pattern, &self.references[*index])
            })
            .collect()
    }
}

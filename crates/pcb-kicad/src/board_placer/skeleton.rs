// Copyright (C) 2026 Toit contributors.

//! The race over skeletons' class rule: which footprints are passives
//! (docs/placer.md, "A race over skeletons (2026-11-04)"), the one place
//! the rule lives. The placement is `pcb_placer::skeleton`'s.

use super::*;
use crate::board_layout::PLACEMENT_SHARE as LAYOUT_PLACEMENT_SHARE;
use pcb_placer as core;
use serde::{Deserialize, Serialize};

/// `KiCadBoardPlacerConfig::skeleton_race`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct KiCadSkeletonRace {
    /// Factor on the area of the passives a skeleton part owns that its
    /// body grows by while the skeleton is placed alone.
    pub room: f64,
    /// Share of a seed's placement work the skeleton may take; the
    /// passives take what is left.
    pub work_share: f64,
    /// A two-pin passive joins its two nets in the skeleton where both have
    /// at most this many pins and no pour carries either (0: none).
    pub join_pins: usize,
    /// Share of the race's seeds placed plain (the whole board at once, as
    /// with the switch off), the others by skeleton: the race then ranks
    /// both kinds (`plain_share`). 0: every seed by skeleton.
    pub plain_share: f64,
    /// The least work of a seed's placement (seconds; every seed of a race
    /// has the placement's) at which the race places by skeleton: below it
    /// every seed places plain, as with the switch off (`budget_ok`). At
    /// the short tier's 200 s the skeleton's half of a seed's work makes
    /// OpenFC's skeleton skip to the tight levels (`skip_to_tight`) on a
    /// quarter of its seeds, and the race keeps those; it walks its levels
    /// on every seed from about 475 s (docs/placer.md, "The skeleton's
    /// budget (2026-11-05)").
    pub min_work_seconds: f64,
}

/// The least placement work at which the race runs its skeletons
/// (`KiCadSkeletonRace::min_work_seconds`).
pub const SKELETON_MIN_WORK_SECONDS: f64 = 500.0;

/// Whether a seed's placement work (`None`: unlimited) lets the race place
/// by skeleton (`KiCadSkeletonRace::min_work_seconds`).
pub(crate) fn budget_ok(work_seconds: Option<f64>, min_work_seconds: f64) -> bool {
    work_seconds.is_none_or(|seconds| seconds >= min_work_seconds)
}

/// Whether seed `offset` of a race over skeletons places plain: the seeds
/// spread `share` of the race evenly, the first by skeleton (offsets 1, 3,
/// 5, ... at 0.5).
pub(crate) fn plain_seed(offset: u64, share: f64) -> bool {
    let share = share.clamp(0.0, 1.0);
    ((offset + 1) as f64 * share).floor() > (offset as f64 * share).floor()
}

impl Default for KiCadSkeletonRace {
    fn default() -> Self {
        let settings = core::skeleton::Settings::default();
        Self {
            room: settings.room,
            work_share: settings.work_share,
            join_pins: settings.join_pins,
            plain_share: 0.0,
            min_work_seconds: SKELETON_MIN_WORK_SECONDS,
        }
    }
}

impl KiCadSkeletonRace {
    pub fn settings(&self) -> core::skeleton::Settings {
        core::skeleton::Settings {
            room: self.room,
            work_share: self.work_share,
            join_pins: self.join_pins,
        }
    }
}

/// The reference prefixes of passives (`partialcopy.py`'s
/// `PASSIVE_PREFIXES`): resistors, capacitors, inductors, ferrite beads,
/// fuses, filters and arrays of them.
const PASSIVE_PREFIXES: [&str; 11] = [
    "R", "C", "L", "FB", "F", "RN", "RA", "RR", "CN_ARRAY", "FL", "FER",
];

/// Of those, the arrays: passives with more than two pads.
const ARRAY_PREFIXES: [&str; 4] = ["RN", "RA", "RR", "CN_ARRAY"];

/// A connector by `metrics.py`'s test: a connector's reference prefix or a
/// connector's word in the footprint's name.
pub(crate) fn skeleton_connector(reference: &str, library: &str) -> bool {
    let prefix = reference.to_ascii_uppercase();
    let prefix = prefix.trim_end_matches(|c: char| c.is_ascii_digit());
    let library = library.to_ascii_lowercase();
    matches!(prefix, "J" | "P" | "CN" | "USB" | "CON" | "X")
        || [
            "connector",
            "conn_",
            "header",
            "usb",
            "jack",
            "socket",
            "terminal",
            "pinheader",
            "pinsocket",
        ]
        .iter()
        .any(|word| library.contains(word))
}

/// A passive's footprint: from a passive library (`Resistor_SMD:...`,
/// `Capacitor_THT:...`, `Inductor_SMD:...`, `Fuse:...`, a ferrite's), or
/// named as one, a size code after the letter (`R_0603_1608Metric`,
/// `C_0402`, `CP_1206`, `L_1206`, `FB_0805`).
fn passive_footprint(library: &str) -> bool {
    let library = library.to_ascii_lowercase();
    let (lib, name) = library.split_once(':').unwrap_or(("", library.as_str()));
    ["resistor", "capacitor", "inductor", "fuse", "ferrite"]
        .iter()
        .any(|word| lib.starts_with(word))
        || ["r_", "c_", "cp_", "l_", "fb_"].iter().any(|start| {
            name.strip_prefix(start)
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
        })
}

/// Whether a footprint is a passive the skeleton leaves out: never a
/// connector; with at most two pads, a passive's reference prefix
/// (`partialcopy.py`'s rule: R, C, L, FB, F, FL, FER) or a passive's
/// footprint (`passive_footprint`, whatever the reference); with more, an
/// array's prefix (RN, RA, RR, CN_ARRAY). Diodes, LEDs, transistors,
/// crystals, a three-pad trimmer, a four-pad choke stay in the skeleton,
/// as `partialcopy.py`'s `nonpassive` rule pinned them.
pub fn skeleton_passive(reference: &str, library: &str, pads: usize) -> bool {
    if skeleton_connector(reference, library) {
        return false;
    }
    let prefix: String = reference
        .chars()
        .take_while(|c| c.is_ascii_alphabetic() || *c == '_')
        .collect::<String>()
        .to_ascii_uppercase();
    let prefix = prefix.as_str();
    if pads <= 2 {
        PASSIVE_PREFIXES.contains(&prefix) || passive_footprint(library)
    } else {
        ARRAY_PREFIXES.contains(&prefix)
    }
}

/// Per component of the lowered problem, whether it is a passive by
/// `skeleton_passive` (components past the board's footprints, its marks
/// and cutouts, are none). The pads are counted by their numbers.
pub(crate) fn passive_mask(pcb: &Expr, components: usize) -> Vec<bool> {
    let mut mask: Vec<bool> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .map(|footprint| {
            let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
            let library = footprint
                .children()
                .get(1)
                .and_then(Expr::atom)
                .unwrap_or_default()
                .to_string();
            let numbers: BTreeSet<&str> = footprint
                .children()
                .iter()
                .filter(|child| child.head() == Some("pad"))
                .filter_map(|pad| pad.children().get(1).and_then(Expr::atom))
                .filter(|number| !number.is_empty())
                .collect();
            skeleton_passive(&reference, &library, numbers.len())
        })
        .collect();
    mask.resize(components, false);
    mask
}

/// The race over skeletons in `board-placer.json`: the class rule's counts
/// and what the kept placement's seed did, with the work of every kept
/// seed's two phases.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct KiCadSkeletonRaceReport {
    /// Whether the placement's work (`work_seconds`, the seed's: every seed
    /// of the race has it) reached `min_work_seconds`: the seeds were
    /// placed by skeleton; else plain, as with the switch off, and the
    /// fields below are the plain race's (no skeleton, every seed plain).
    pub budget_ok: bool,
    pub work_seconds: Option<f64>,
    pub min_work_seconds: f64,
    /// Footprints the class rule calls passives, and those the skeleton
    /// left out (the rest a constraint names, or fixed).
    pub passives: usize,
    pub absent: usize,
    /// The kept placement: the movable parts in its skeleton, those held
    /// while the passives were placed, the nets joined through passives,
    /// the parts grown and the room they took (mm²), the passives' area no
    /// part took, the first level at which the whole board fits by area
    /// (the skeleton's first), the levels of the two phases, the skeleton's wirelength
    /// and whether the fallback (a whole placement) ran and was kept.
    pub skeleton_parts: usize,
    pub held: usize,
    pub joined_nets: usize,
    pub grown: usize,
    pub room_mm2: f64,
    pub unowned_mm2: f64,
    pub fitting_level: usize,
    pub skeleton_level: usize,
    pub level: usize,
    pub skeleton_wirelength_mm: f64,
    pub fallback: bool,
    pub fallback_kept: bool,
    /// The kept placement's work: the skeleton's, the passives', the
    /// fallback's (seconds of an idle machine).
    pub skeleton_work_seconds: f64,
    pub passive_work_seconds: f64,
    pub fallback_work_seconds: f64,
    /// Over every kept seed: the same three, and how many seeds fell back.
    pub seeds_skeleton_work_seconds: f64,
    pub seeds_passive_work_seconds: f64,
    pub seeds_fallback_work_seconds: f64,
    pub seeds_fallback: usize,
    /// Of the kept seeds, those placed plain (`plain_share`), and whether
    /// the kept placement is one (then the fields above describing the
    /// kept placement's skeleton are zero).
    pub seeds_plain: usize,
    pub kept_plain: bool,
    /// What the race did, in one line (also on stderr as `placer: skeleton
    /// race: ...`): whether the seeds placed by skeleton, and if not why
    /// and what would turn it on; else what the kept seed did (its levels,
    /// a fallback) and how many seeds fell back.
    #[serde(default)]
    pub summary: String,
}

impl KiCadSkeletonRaceReport {
    /// `summary`: one line on what ran (trial 7: a short budget's race was
    /// told only by `budget_ok: false` and zero levels, which read as
    /// "seated at full halos").
    fn summarize(&self, seeds: usize) -> String {
        if !self.budget_ok {
            let work = self.work_seconds.unwrap_or(0.0);
            return format!(
                "not run: each seed's placement had {work:.0} s of work, under min_work_seconds \
                 ({:.0} s), so all {seeds} seeds placed plain, as with \"skeleton_race\": null \
                 (its level fields are 0 for that reason); to place by skeleton give the \
                 placement {:.0} s or more: a layout's total_seconds of {:.0} or more (the \
                 placement takes {:.0} % of it; a place check takes its layout.json's), or \
                 lower skeleton_race.min_work_seconds",
                self.min_work_seconds,
                self.min_work_seconds,
                (self.min_work_seconds / LAYOUT_PLACEMENT_SHARE).ceil(),
                LAYOUT_PLACEMENT_SHARE * 100.0,
            );
        }
        let by_skeleton = seeds - self.seeds_plain;
        let seeds_line = format!(
            "{by_skeleton} of {seeds} seeds placed by skeleton ({} fell back to a plain \
             placement){}",
            self.seeds_fallback,
            if self.seeds_plain > 0 {
                format!(", {} plain (plain_share)", self.seeds_plain)
            } else {
                String::new()
            }
        );
        let levels = "levels: 0 is full halos, each further level shrinks halos and spacing, \
                      the last ones the bodies (tight)";
        if self.kept_plain {
            return format!(
                "ran: the kept seed is one placed plain (plain_share); {seeds_line}; the fields \
                 of the kept seed's skeleton are 0"
            );
        }
        if self.absent == 0 && !self.fallback {
            return format!(
                "ran without a skeleton: none of the {} passives was free (a constraint the user \
                 wrote names each, or it is fixed), so the kept seed placed the whole board at \
                 once (level {}) with the race's global settings; {seeds_line}",
                self.passives, self.level
            );
        }
        let kept = if self.fallback_kept {
            format!(
                "the kept seed's skeleton ({} parts, level {}) or its passives left parts \
                 without a place, so it was placed plain (fallback_kept)",
                self.skeleton_parts, self.skeleton_level
            )
        } else {
            format!(
                "the kept seed placed its skeleton of {} parts first (level {}) and its {} \
                 passives around it (level {}{})",
                self.skeleton_parts,
                self.skeleton_level,
                self.absent,
                self.level,
                if self.fallback {
                    "; a fallback ran and was not kept"
                } else {
                    ""
                }
            )
        };
        format!("ran: {kept}; {seeds_line}; {levels}")
    }
}

impl KiCadSkeletonRaceReport {
    pub(crate) fn of(
        race: &KiCadSkeletonRace,
        work_seconds: Option<f64>,
        passive: &[bool],
        kept: &core::Placement,
        seeds: &[&core::Placement],
    ) -> Self {
        let seconds = |work: u64| work as f64 / core::work_per_second();
        let report = kept.skeleton.clone().unwrap_or_default();
        let reports: Vec<core::skeleton::Report> = seeds
            .iter()
            .filter_map(|placement| placement.skeleton.clone())
            .collect();
        Self {
            budget_ok: budget_ok(work_seconds, race.min_work_seconds),
            work_seconds,
            min_work_seconds: race.min_work_seconds,
            passives: passive.iter().filter(|passive| **passive).count(),
            absent: report.absent,
            skeleton_parts: report.skeleton_parts,
            held: report.held,
            joined_nets: report.joined_nets,
            grown: report.grown,
            room_mm2: report.room_mm2,
            unowned_mm2: report.unowned_mm2,
            fitting_level: report.fitting_level,
            skeleton_level: report.skeleton_level,
            level: report.level,
            skeleton_wirelength_mm: report.skeleton_wirelength,
            fallback: report.fallback,
            fallback_kept: report.fallback_kept,
            skeleton_work_seconds: seconds(report.skeleton_work),
            passive_work_seconds: seconds(report.passive_work),
            fallback_work_seconds: seconds(report.fallback_work),
            seeds_skeleton_work_seconds: seconds(reports.iter().map(|r| r.skeleton_work).sum()),
            seeds_passive_work_seconds: seconds(reports.iter().map(|r| r.passive_work).sum()),
            seeds_fallback_work_seconds: seconds(reports.iter().map(|r| r.fallback_work).sum()),
            seeds_fallback: reports.iter().filter(|r| r.fallback).count(),
            seeds_plain: seeds.iter().filter(|seed| seed.skeleton.is_none()).count(),
            kept_plain: kept.skeleton.is_none(),
            summary: String::new(),
        }
        .with_summary(seeds.len())
    }

    fn with_summary(mut self, seeds: usize) -> Self {
        self.summary = self.summarize(seeds);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_seeds_spread_their_share_over_the_race() {
        let plain = |share: f64| -> Vec<bool> { (0..6).map(|k| plain_seed(k, share)).collect() };
        assert_eq!(plain(0.0), [false; 6]);
        assert_eq!(plain(0.5), [false, true, false, true, false, true]);
        assert_eq!(plain(1.0), [true; 6]);
        assert_eq!(plain(1.0 / 3.0), [false, false, true, false, false, true]);
    }

    #[test]
    fn two_pad_passives_by_reference_or_footprint() {
        // By reference prefix, two pads.
        assert!(skeleton_passive("R12", "Resistor_SMD:R_0603_1608Metric", 2));
        assert!(skeleton_passive("C3", "Capacitor_SMD:C_0402_1005Metric", 2));
        assert!(skeleton_passive("L1", "Inductor_SMD:L_1210", 2));
        assert!(skeleton_passive("FB2", "Inductor_SMD:L_0603", 2));
        assert!(skeleton_passive("F1", "Fuse:Fuse_1206_3216Metric", 2));
        // By footprint, whatever the reference.
        assert!(skeleton_passive("Z5", "Resistor_SMD:R_0402_1005Metric", 2));
        assert!(skeleton_passive("PU1", "myparts:R_0603", 2));
        assert!(skeleton_passive("CX9", "Capacitor_SMD:CP_Elec_5x5.4", 2));
        assert!(skeleton_passive("E2", "lib:CP_1206", 2));
        // Arrays by their prefix.
        assert!(skeleton_passive(
            "RN1",
            "Resistor_SMD:R_Array_Convex_4x0603",
            8
        ));
        assert!(skeleton_passive("CN_ARRAY1", "lib:C_Array", 8));
    }

    /// Two 8-pin chips on a 40 x 30 mm board, four series resistors
    /// between them and a decoupling capacitor at each.
    fn board() -> String {
        let name = |net: usize| match net {
            9 => "+3V3".to_string(),
            10 => "GND".to_string(),
            net => format!("N{net}"),
        };
        let pad = |number: usize, x: f64, y: f64, net: usize| {
            format!(
                r#"(pad "{number}" smd rect (at {x} {y}) (size 0.6 0.9) (layers "F.Cu") (net {net} "{}"))"#,
                name(net)
            )
        };
        let part = |library: &str, reference: &str, at: [f64; 2], half: [f64; 2], pads: String| {
            format!(
                r#"(footprint "{library}" (layer "F.Cu") (at {} {}) (property "Reference" "{reference}") (attr smd)
                    (fp_rect (start {} {}) (end {} {}) (layer "F.CrtYd")) {pads})"#,
                at[0], at[1], -half[0], -half[1], half[0], half[1]
            )
        };
        let chip = |first: usize| -> String {
            (1..=8)
                .map(|pin| {
                    let (x, y) = if pin <= 4 {
                        (-2.5, pin as f64 - 2.5)
                    } else {
                        (2.5, 6.5 - pin as f64)
                    };
                    let net = match pin {
                        1..=4 => first + pin - 1,
                        5 => 9,
                        _ => 10,
                    };
                    pad(pin, x, y, net)
                })
                .collect()
        };
        let mut footprints = vec![
            part("Package_SO:SOIC-8", "U1", [20.0, 15.0], [3.5, 2.5], chip(1)),
            part("Package_SO:SOIC-8", "U2", [20.0, 15.0], [3.5, 2.5], chip(5)),
        ];
        for index in 1..=4 {
            footprints.push(part(
                "Resistor_SMD:R_0603_1608Metric",
                &format!("R{index}"),
                [20.0, 15.0],
                [1.5, 0.7],
                pad(1, -0.8, 0.0, index) + &pad(2, 0.8, 0.0, index + 4),
            ));
        }
        for index in 1..=2 {
            footprints.push(part(
                "Capacitor_SMD:C_0402_1005Metric",
                &format!("C{index}"),
                [20.0, 15.0],
                [1.0, 0.5],
                pad(1, -0.5, 0.0, 9) + &pad(2, 0.5, 0.0, 10),
            ));
        }
        let nets: String = (1..=10)
            .map(|net| format!(r#"(net {net} "{}")"#, name(net)))
            .collect();
        format!(
            r#"(kicad_pcb (version 20240108) (layers (0 "F.Cu" signal) (31 "B.Cu" signal)) (net 0 "") {nets}
                (gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts")) {})"#,
            footprints.concat()
        )
    }

    #[test]
    fn a_race_over_skeletons_places_and_reports() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("pcb-maker-skeleton-{}-{nonce}", std::process::id()));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("board.kicad_pcb"), board()).unwrap();
        let mut results = Vec::new();
        for (name, race) in [("off", None), ("on", Some(KiCadSkeletonRace::default()))] {
            let config = KiCadBoardPlacerConfig {
                placement_seeds: 2,
                skeleton_race: race,
                ..KiCadBoardPlacerConfig::default()
            };
            let result = place_kicad_board(&source, "board", &root.join(name), &config).unwrap();
            assert!(
                result.unplaced.is_empty() && result.illegal.is_empty(),
                "{name}: {:?} {:?}",
                result.unplaced,
                result.illegal
            );
            results.push(result);
        }
        let _ = fs::remove_dir_all(&root);
        assert!(results[0].skeleton_race.is_none());
        let report = results[1].skeleton_race.clone().expect("the race's report");
        // No work limit: the race places by skeleton.
        assert!(report.budget_ok && report.work_seconds.is_none());
        // R1-R4 and C1, C2 are the passives; the two chips the skeleton,
        // both held; the four series resistors' nets joined.
        assert_eq!(report.passives, 6);
        assert_eq!(report.absent, 6);
        assert_eq!((report.skeleton_parts, report.held), (2, 2));
        assert_eq!(report.joined_nets, 4);
        assert_eq!(report.grown, 2);
        assert!(!report.fallback_kept);
        // The kept seed's work is its two phases'; the seeds' sums hold it.
        assert!(report.skeleton_work_seconds > 0.0 && report.passive_work_seconds > 0.0);
        let phases = report.seeds_skeleton_work_seconds
            + report.seeds_passive_work_seconds
            + report.seeds_fallback_work_seconds;
        assert!((phases - results[1].seeds_work_seconds).abs() < 1e-6);
        assert_eq!(results[1].seeds, 2);
    }

    #[test]
    fn a_board_in_pieces_still_has_a_skeleton() {
        // The board beside an empty second piece (zpn_devboard's two
        // pieces), J1 locked on the first on the rail and ground: every
        // movable part joins J1 by its nets and the lowering holds it on
        // J1's piece (`keep_on_pieces`), a region the user did not write.
        // The passives are still left out of the skeleton.
        let j1 = r#"(footprint "Connector_PinHeader:PinHeader_1x02" (layer "F.Cu") (at 3 15) (locked yes) (property "Reference" "J1") (attr through_hole)
                (fp_rect (start -1.5 -2.5) (end 1.5 2.5) (layer "F.CrtYd"))
                (pad "1" smd rect (at 0 -1.27) (size 1.7 1.7) (layers "F.Cu") (net 9 "+3V3"))
                (pad "2" smd rect (at 0 1.27) (size 1.7 1.7) (layers "F.Cu") (net 10 "GND")))"#;
        let pcb = board().replacen(
            r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts"))"#,
            &format!(
                r#"(gr_rect (start 0 0) (end 40 30) (layer "Edge.Cuts"))
                   (gr_rect (start 50 0) (end 70 30) (layer "Edge.Cuts")) {j1}"#
            ),
            1,
        );
        let lowered =
            lower_placement(&parse(&pcb).unwrap(), &KiCadBoardPlacerConfig::default()).unwrap();
        let constraints = &lowered.problem.constraints;
        assert_eq!(lowered.problem.pieces.len(), 2);
        assert_eq!(constraints.regions.len(), 8);
        assert_eq!(constraints.automatic_regions.len(), 8);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pcb-maker-skeleton-pieces-{}-{nonce}",
            std::process::id()
        ));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("board.kicad_pcb"), &pcb).unwrap();
        let config = KiCadBoardPlacerConfig {
            placement_seeds: 2,
            skeleton_race: Some(KiCadSkeletonRace::default()),
            ..KiCadBoardPlacerConfig::default()
        };
        let result = place_kicad_board(&source, "board", &root.join("on"), &config).unwrap();
        let _ = fs::remove_dir_all(&root);
        assert!(result.unplaced.is_empty() && result.illegal.is_empty());
        let report = result.skeleton_race.expect("the race's report");
        assert_eq!((report.passives, report.absent), (6, 6));
        assert_eq!((report.skeleton_parts, report.held), (2, 2));
        // The chips own the passives' room: a piece's region is no reason
        // to keep a body as it is.
        assert_eq!(report.grown, 2);
        assert!(
            report
                .summary
                .starts_with("ran: the kept seed placed its skeleton of 2 parts first"),
            "{}",
            report.summary
        );
    }

    #[test]
    fn the_race_places_by_skeleton_from_its_least_work_on() {
        assert!(budget_ok(None, SKELETON_MIN_WORK_SECONDS));
        assert!(budget_ok(Some(600.0), SKELETON_MIN_WORK_SECONDS));
        assert!(budget_ok(Some(500.0), 500.0));
        assert!(!budget_ok(Some(499.9), 500.0));
        // The short tier's placement (a third of the layout's 600 s).
        assert!(!budget_ok(Some(200.0), SKELETON_MIN_WORK_SECONDS));
        assert!(budget_ok(Some(200.0), 0.0));
        let race: KiCadSkeletonRace = serde_json::from_str("{}").unwrap();
        assert_eq!(race.min_work_seconds, SKELETON_MIN_WORK_SECONDS);
    }

    #[test]
    fn below_its_least_work_the_race_places_plain() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pcb-maker-skeleton-budget-{}-{nonce}",
            std::process::id()
        ));
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("board.kicad_pcb"), board()).unwrap();
        let place = |name: &str, race: Option<KiCadSkeletonRace>| {
            let config = KiCadBoardPlacerConfig {
                placement_seeds: 2,
                work_seconds: Some(30.0),
                skeleton_race: race,
                ..KiCadBoardPlacerConfig::default()
            };
            let result = place_kicad_board(&source, "board", &root.join(name), &config).unwrap();
            let board = fs::read_to_string(root.join(name).join("board.kicad_pcb")).unwrap();
            (result, board)
        };
        let (off, off_board) = place("off", None);
        let (short, short_board) = place("short", Some(KiCadSkeletonRace::default()));
        let (on, _) = place(
            "on",
            Some(KiCadSkeletonRace {
                min_work_seconds: 30.0,
                ..KiCadSkeletonRace::default()
            }),
        );
        let _ = fs::remove_dir_all(&root);
        assert!(off.skeleton_race.is_none());
        // 30 s of work, under the race's 500: every seed plain, the board
        // the switch off writes.
        let report = short.skeleton_race.expect("the race's report");
        assert!(!report.budget_ok);
        assert_eq!(report.work_seconds, Some(30.0));
        assert!(report.kept_plain && report.seeds_plain == short.seeds);
        assert_eq!(report.skeleton_parts, 0);
        assert_eq!(short_board, off_board);
        // At its least work the race places by skeleton.
        let report = on.skeleton_race.expect("the race's report");
        assert!(report.budget_ok && !report.kept_plain);
        assert_eq!((report.skeleton_parts, report.held), (2, 2));
    }

    #[test]
    fn the_race_says_in_one_line_what_ran_and_why_not() {
        // Trial 7's MIDAS-MK2 at total_seconds 500: 200 s a seed, plain.
        let short = KiCadSkeletonRaceReport {
            budget_ok: false,
            work_seconds: Some(200.0),
            min_work_seconds: SKELETON_MIN_WORK_SECONDS,
            passives: 131,
            seeds_plain: 3,
            kept_plain: true,
            ..KiCadSkeletonRaceReport::default()
        }
        .with_summary(3);
        assert!(
            short.summary.starts_with(
                "not run: each seed's placement had 200 s of work, under min_work_seconds \
                 (500 s), so all 3 seeds placed plain"
            ),
            "{}",
            short.summary
        );
        // The lever: the layout's budget that gives the placement 500 s.
        assert!(
            short.summary.contains("total_seconds of 1250 or more"),
            "{}",
            short.summary
        );
        // At the default budget: the kept seed's skeleton and levels.
        let ran = KiCadSkeletonRaceReport {
            budget_ok: true,
            work_seconds: Some(600.0),
            min_work_seconds: SKELETON_MIN_WORK_SECONDS,
            passives: 131,
            absent: 129,
            skeleton_parts: 68,
            held: 68,
            skeleton_level: 3,
            level: 3,
            seeds_fallback: 1,
            ..KiCadSkeletonRaceReport::default()
        }
        .with_summary(12);
        assert!(
            ran.summary.starts_with(
                "ran: the kept seed placed its skeleton of 68 parts first (level 3) and its 129 \
                 passives around it (level 3); 12 of 12 seeds placed by skeleton (1 fell back \
                 to a plain placement)"
            ),
            "{}",
            ran.summary
        );
        let fell_back = KiCadSkeletonRaceReport {
            fallback: true,
            fallback_kept: true,
            ..ran.clone()
        }
        .with_summary(12);
        assert!(
            fell_back
                .summary
                .contains("so it was placed plain (fallback_kept)")
        );
        // No passive free of the user's constraints: no skeleton, said so.
        let whole = KiCadSkeletonRaceReport {
            absent: 0,
            skeleton_parts: 0,
            held: 0,
            skeleton_level: 0,
            level: 2,
            seeds_fallback: 0,
            ..ran.clone()
        }
        .with_summary(12);
        assert!(
            whole.summary.starts_with(
                "ran without a skeleton: none of the 131 passives was free (a constraint the \
                 user wrote names each, or it is fixed), so the kept seed placed the whole \
                 board at once (level 2)"
            ),
            "{}",
            whole.summary
        );
        assert!(!whole.summary.contains("skeleton of 0 parts"));
        // The serialized report carries it.
        let json = serde_json::to_value(&ran).unwrap();
        assert_eq!(json["summary"], ran.summary.as_str());
    }

    #[test]
    fn the_rest_stays_in_the_skeleton() {
        // Diodes and LEDs (partialcopy.py's nonpassive pinned them).
        assert!(!skeleton_passive("D1", "Diode_SMD:D_SOD-123", 2));
        assert!(!skeleton_passive("LED3", "LED_SMD:LED_0603_1608Metric", 2));
        // ICs, transistors, crystals.
        assert!(!skeleton_passive("U1", "Package_QFP:LQFP-48", 48));
        assert!(!skeleton_passive("Q2", "Package_TO_SOT_SMD:SOT-23", 3));
        assert!(!skeleton_passive("Y1", "Crystal:Crystal_SMD_3225-4Pin", 4));
        // A passive's letter with more pads: a trimmer, a choke.
        assert!(!skeleton_passive("R5", "Potentiometer_SMD:Trimmer", 3));
        assert!(!skeleton_passive("L2", "Inductor_SMD:L_CommonMode", 4));
        // Connectors, also with a passive's letter or footprint word.
        assert!(!skeleton_passive(
            "J1",
            "Connector_PinHeader:PinHeader_1x02",
            2
        ));
        assert!(!skeleton_passive("C9", "Connector_JST:JST_PH_2", 2));
        assert!(!skeleton_passive("CN1", "lib:R_0603", 2));
        // A netless-looking test point or a jumper is not a passive.
        assert!(!skeleton_passive(
            "TP1",
            "TestPoint:TestPoint_Pad_1.0x1.0mm",
            1
        ));
        assert!(!skeleton_passive("JP1", "Jumper:SolderJumper_2_Open", 2));
    }
}

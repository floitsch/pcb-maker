// Copyright (C) 2026 Toit contributors.

//! The race over skeletons' class rule: which footprints are passives
//! (docs/placer.md, "A race over skeletons (2026-11-04)"), the one place
//! the rule lives. The placement is `pcb_placer::skeleton`'s.

use super::*;
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
    /// both kinds (`plain_seed`). 0: every seed by skeleton.
    pub plain_share: f64,
    /// The level guard (docs/placer.md, "The level guard (2026-11-05)"):
    /// the race's first seed is also placed plain beside the seeds (a
    /// probe, not among them), and its relaxation level bounds both phases
    /// of every skeleton seed. `walk`: no phase walks to a tighter level; a
    /// skeleton or passives that would seat only there fall back to the
    /// plain placement. `skips`: no phase skips to the tight levels
    /// (`skip_to_tight`, which a phase's share of the work triggers) past
    /// it; the walk level by level is free. `off` (the default): no probe,
    /// no bound. Measured on OpenFC and protoesp and not taken: both modes
    /// lose OpenFC's strict pass on some seeds and `skips` gives up
    /// protoesp's gain, which is the skip to the tight levels.
    pub level_guard: KiCadLevelGuard,
}

/// `KiCadSkeletonRace::level_guard`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KiCadLevelGuard {
    #[default]
    Off,
    Skips,
    Walk,
}

impl KiCadLevelGuard {
    /// The bound the race's probe sets, where the guard is on.
    pub(crate) fn bound(self) -> Option<core::LevelBound> {
        match self {
            Self::Off => None,
            Self::Skips => Some(core::LevelBound::pending().skips_only()),
            Self::Walk => Some(core::LevelBound::pending()),
        }
    }
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
            level_guard: KiCadLevelGuard::default(),
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
    /// The level guard (`level_guard`): the plain probe's relaxation level
    /// (the bound; `None` without the guard) and work (seconds), the kept
    /// seeds whose skeleton or passives stopped at the bound and fell back
    /// to the plain placement, and whether the kept placement's did.
    pub plain_level: Option<usize>,
    pub probe_work_seconds: f64,
    pub seeds_bounded: usize,
    pub bounded: bool,
}

impl KiCadSkeletonRaceReport {
    pub(crate) fn of(
        passive: &[bool],
        kept: &core::Placement,
        seeds: &[&core::Placement],
        probe: Option<(usize, u64)>,
    ) -> Self {
        let seconds = |work: u64| work as f64 / core::work_per_second();
        let report = kept.skeleton.clone().unwrap_or_default();
        let reports: Vec<core::skeleton::Report> = seeds
            .iter()
            .filter_map(|placement| placement.skeleton.clone())
            .collect();
        Self {
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
            plain_level: probe.map(|(level, _)| level),
            probe_work_seconds: probe.map_or(0.0, |(_, work)| seconds(work)),
            seeds_bounded: reports.iter().filter(|r| r.bounded).count(),
            bounded: report.bounded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_guard_is_off_by_default_and_reported() {
        let race: KiCadSkeletonRace = serde_json::from_str("{}").expect("the defaults");
        assert_eq!(race.level_guard, KiCadLevelGuard::Off);
        assert!(race.level_guard.bound().is_none());
        assert!(
            KiCadLevelGuard::Skips
                .bound()
                .is_some_and(|bound| !bound.walk)
        );
        for (text, guard) in [
            ("off", KiCadLevelGuard::Off),
            ("skips", KiCadLevelGuard::Skips),
            ("walk", KiCadLevelGuard::Walk),
        ] {
            let race: KiCadSkeletonRace =
                serde_json::from_str(&format!(r#"{{"level_guard": "{text}"}}"#)).expect(text);
            assert_eq!(race.level_guard, guard);
        }
        assert!(
            KiCadLevelGuard::Walk
                .bound()
                .is_some_and(|bound| bound.walk)
        );
        let mut kept = core::Placement {
            poses: Vec::new(),
            frames: Vec::new(),
            global_iterations: 0,
            global_overflow: 0.0,
            wirelength_initial: 0.0,
            wirelength_global: 0.0,
            wirelength_legal: 0.0,
            wirelength_final: 0.0,
            unplaced: Vec::new(),
            illegal: Vec::new(),
            constraints: Vec::new(),
            relaxation: core::Relaxation {
                spacing: 0.0,
                grid: 0.1,
                halo_scale: 0.0,
                small_halo_scale: 0.0,
                edge_inset: false,
                tight: false,
                edge_copper: false,
                edge_rule: 0.0,
                courtyard_spacing: false,
                pad_clearance: false,
                halos: None,
                on_marks: Vec::new(),
            },
            work: 0,
            stage_poses: Vec::new(),
            overfull: false,
            exits: None,
            level: 4,
            skeleton: Some(core::skeleton::Report {
                bounded: true,
                fallback: true,
                fallback_kept: true,
                ..Default::default()
            }),
            bounded: false,
        };
        let mut other = kept.clone();
        other.skeleton = Some(core::skeleton::Report::default());
        let work = (2.0 * core::work_per_second()) as u64;
        let report =
            KiCadSkeletonRaceReport::of(&[true, false], &kept, &[&kept, &other], Some((4, work)));
        assert_eq!(report.plain_level, Some(4));
        assert!((report.probe_work_seconds - 2.0).abs() < 1e-9);
        assert_eq!(report.seeds_bounded, 1);
        assert!(report.bounded);
        kept.skeleton = None;
        let unguarded = KiCadSkeletonRaceReport::of(&[true], &kept, &[&kept], None);
        assert_eq!(unguarded.plain_level, None);
        assert_eq!(unguarded.seeds_bounded, 0);
        assert!(!unguarded.bounded && unguarded.kept_plain);
    }

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

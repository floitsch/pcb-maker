// Copyright (C) 2026 Toit contributors.

//! The fixture test of the placement that needs the crate's insides, on
//! the committed fixture board F1 (`tests/fixtures/f1`; docs/handover.md,
//! "The fixture boards"): the order test without the race (test 9). The
//! lowering's fixture tests are pcb-kicad-route's.

use super::*;

/// The repository's `tests/fixtures`.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn f1_text() -> String {
    fs::read_to_string(fixtures().join("f1/f1.kicad_pcb")).unwrap()
}

/// F1 with its footprints, and each footprint's pads and graphics, in the
/// other order.
fn reversed_f1() -> String {
    let mut pcb = parse(&f1_text()).unwrap();
    let Expr::List(items) = &mut pcb else {
        panic!("a list")
    };
    let places: Vec<usize> = (0..items.len())
        .filter(|index| items[*index].head() == Some("footprint"))
        .collect();
    let mut footprints: Vec<Expr> = places.iter().map(|index| items[*index].clone()).collect();
    footprints.reverse();
    for (place, mut footprint) in places.into_iter().zip(footprints) {
        if let Expr::List(parts) = &mut footprint {
            for kind in ["pad", "fp_"] {
                let slots: Vec<usize> = (0..parts.len())
                    .filter(|index| {
                        parts[*index]
                            .head()
                            .is_some_and(|head| head.starts_with(kind))
                    })
                    .collect();
                let mut moved: Vec<Expr> =
                    slots.iter().map(|index| parts[*index].clone()).collect();
                moved.reverse();
                for (slot, part) in slots.into_iter().zip(moved) {
                    parts[slot] = part;
                }
            }
        }
        items[place] = footprint;
    }
    encode(&pcb)
}

#[test]
#[ignore = "until fence-report-placer's fix of canonical_order's SOURCE_ORDERS lands (review 2026-10-21, fixture test 9)"]
fn f1_places_the_same_in_either_order_of_the_file() {
    // Fixture test 9: the order test without the race.
    // board_placer's a_placement_does_not_depend_on_the_order_of_the_file
    // compares the written files, whose footprint graphics the writer
    // puts back in the order of the file read last by any thread
    // (`canonical_order::SOURCE_ORDERS`); here the placements of F1 and
    // of F1 reversed run one after the other and their boards are
    // compared as `parse_board` reads them back (canonical order), with
    // the poses and wirelengths.
    let forward = f1_text();
    let backward = reversed_f1();
    assert_ne!(forward, backward);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("pcb-maker-f1-order-{}-{nonce}", std::process::id()));
    let config = KiCadBoardPlacerConfig {
        fixed: vec!["J1".into(), "H1".into()],
        tuning: crate::KiCadPlacerTuning {
            placement_seeds: 2,
            ..Default::default()
        },
        ..KiCadBoardPlacerConfig::default()
    };
    let mut placed = Vec::new();
    for (name, text) in [("forward", &forward), ("backward", &backward)] {
        let source = root.join(format!("{name}-source"));
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("f1.kicad_pcb"), text).unwrap();
        fs::copy(
            fixtures().join("f1/f1.kicad_pro"),
            source.join("f1.kicad_pro"),
        )
        .unwrap();
        let output = root.join(format!("{name}-placed"));
        let result = place_kicad_board(&source, "f1", &output, &config).unwrap();
        let written = fs::read_to_string(output.join("f1.kicad_pcb")).unwrap();
        let canonical = encode(&crate::canonical_order::parse_board(&written).unwrap());
        placed.push((
            result.wirelength_final_mm,
            result.unplaced.clone(),
            result.illegal.clone(),
            canonical,
        ));
    }
    let _ = fs::remove_dir_all(&root);
    assert_eq!(placed[0].0, placed[1].0, "the wirelengths differ");
    assert_eq!(placed[0].1, placed[1].1);
    assert_eq!(placed[0].2, placed[1].2);
    assert!(
        placed[0].3 == placed[1].3,
        "the placed boards differ as parse_board reads them"
    );
}

// Copyright (C) 2026 Toit contributors.

//! The fixture tests that need the crate's insides, on the committed
//! fixture board F1 (`tests/fixtures/f1`; docs/handover.md, "The fixture
//! boards"): the static maps' cache key (review 2026-10-21, fixture test 2), the lowering's
//! obstacles, terminals and no-via pads against a golden table (test 7)
//! and the order test without the race (test 9). The CLI's fixture tests
//! are the root package's `tests/fixtures.rs`; the golden files are
//! shared (`tests/fixtures/golden`, rewritten under `PCB_UPDATE_GOLDEN=1`).

use super::*;
use crate::board_router::{core_config, lower};
use pcb_router::router::{Router, hold_static_maps};

/// The repository's `tests/fixtures`.
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn f1_text() -> String {
    fs::read_to_string(fixtures().join("f1/f1.kicad_pcb")).unwrap()
}

/// F1's router config as `auto` resolves it (the project's classes, its
/// .kicad_dru's rules).
fn f1_config() -> KiCadBoardRouterConfig {
    resolve_project_rules(
        &fixtures().join("f1/f1.kicad_pro"),
        &fixtures().join("f1/f1.kicad_pcb"),
    )
    .unwrap()
}

fn f1_board(config: &KiCadBoardRouterConfig) -> pcb_router::Board {
    let pcb = crate::canonical_order::parse_board(&f1_text()).unwrap();
    lower(&pcb, config, true).unwrap().board
}

/// Compares `actual` with the golden file `name`, or writes it under
/// `PCB_UPDATE_GOLDEN=1`. A difference fails with the lines that differ
/// and how to accept them.
fn golden(name: &str, actual: &str) {
    let path = fixtures().join("golden").join(name);
    if std::env::var("PCB_UPDATE_GOLDEN").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, actual).unwrap();
        eprintln!("golden file written: {}", path.display());
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "golden file {} is missing; write it with PCB_UPDATE_GOLDEN=1",
            path.display()
        )
    });
    if expected != actual {
        let old: BTreeSet<&str> = expected.lines().collect();
        let new: BTreeSet<&str> = actual.lines().collect();
        let mut diff = String::new();
        for line in expected.lines().filter(|line| !new.contains(line)) {
            diff.push_str(&format!("- {line}\n"));
        }
        for line in actual.lines().filter(|line| !old.contains(line)) {
            diff.push_str(&format!("+ {line}\n"));
        }
        if diff.is_empty() {
            diff.push_str("(the same lines in another order)\n");
        }
        panic!(
            "{} differs from its golden file ({}; - golden, + now):\n{diff}\
             If the change is intended, rewrite it with\n  \
             PCB_UPDATE_GOLDEN=1 cargo test --release -p pcb-kicad fixture_tests\n\
             and commit the golden file's diff with the change.",
            name,
            path.display()
        );
    }
}

/// The work of building a router's maps, and of building them alone:
/// equal when the router built its own static maps (none of the cache's
/// fitted its inputs), smaller when it took another router's.
fn build_work(board: &pcb_router::Board, config: &pcb_router::Config) -> (f64, f64) {
    let shared = Router::new(board, config).build_work();
    let alone = Router::new(
        board,
        &pcb_router::Config {
            share_static_maps: false,
            ..config.clone()
        },
    )
    .build_work();
    (shared, alone)
}

#[test]
fn routers_of_other_inputs_do_not_share_static_maps() {
    // Review 2026-10-21, fixture test 2: the static maps are shared by a
    // key of what they are built from (`statics_key`); ded766d had to add
    // `no_via_pads` to it. Routers of F1 that differ in one input hold
    // their own maps, never those of the router held.
    let config = f1_config();
    assert!(
        config.net_rules.is_some(),
        "F1's .kicad_dru gives net rules"
    );
    let base = f1_board(&config);
    let core = core_config(&config);
    let (_held, built) = hold_static_maps(&base, &core);
    // The cache works: a router of the same inputs takes the held maps.
    let (shared, alone) = build_work(&base, &core);
    assert!(
        shared < alone - 1.0e-9,
        "a router of the held inputs shares their maps ({shared} against {alone}, held {built})"
    );
    let differ = |what: &str, board: &pcb_router::Board, core: &pcb_router::Config| {
        let (shared, alone) = build_work(board, core);
        assert!(
            (shared - alone).abs() < 1.0e-12,
            "{what}: the router took the held maps of other inputs ({shared} against {alone} built alone)"
        );
    };
    // Vias in pads allowed: no pad is closed to vias.
    let via_in_pad = KiCadBoardRouterConfig {
        via_in_pad: Some(true),
        ..config.clone()
    };
    let open = f1_board(&via_in_pad);
    assert!(open.no_via_pads.is_empty());
    assert!(!base.no_via_pads.is_empty());
    differ("via_in_pad", &open, &core_config(&via_in_pad));
    // The no-via pads alone (the rest of the board the same).
    let mut fewer = base.clone();
    fewer.no_via_pads.pop();
    differ("one no-via pad fewer", &fewer, &core);
    // Without the .kicad_dru's rules (its PA to PB pair rule).
    let without_rules = KiCadBoardRouterConfig {
        net_rules: None,
        ..config.clone()
    };
    differ(
        "net rules",
        &f1_board(&without_rules),
        &core_config(&without_rules),
    );
    // A keepout more.
    let mut keepout = base.clone();
    keepout.obstacles.push(pcb_router::Obstacle {
        shape: pcb_router::Shape::Circle {
            center: [5.0, 5.0],
            radius: 0.5,
        },
        layers: 1,
        kind: pcb_router::ObstacleKind::Keepout,
        net: None,
        clearance: 0.0,
        clearance_override: None,
        blocks_tracks: true,
        blocks_vias: true,
        label: "test keepout".into(),
    });
    differ("a keepout", &keepout, &core);
    // Another pitch.
    let mut pitch = core.clone();
    pitch.pitches = vec![core.pitches[0] * 0.75];
    differ("the pitch", &base, &pitch);
    // Another class clearance.
    let mut classes = base.clone();
    classes.classes[0].clearance += 0.05;
    differ("a class", &classes, &core);
    // The hole-to-hole spacing.
    let mut holes = base.clone();
    holes.hole_to_hole += 0.1;
    differ("hole_to_hole", &holes, &core);
}

/// An obstacle's label without its part's reference: what kind of item
/// lowered it (`U1.3 mask aperture`: `pad mask aperture`).
fn obstacle_category(label: &str) -> String {
    let mut words = label.split(' ');
    let first = words.next().unwrap_or("");
    let rest: Vec<&str> = words.collect();
    match first.split_once('.') {
        // A pad: `<reference>.<number>`, after it what of the pad.
        Some((reference, _))
            if !reference.is_empty()
                && reference.chars().next().unwrap().is_ascii_uppercase()
                && reference.chars().any(|c| c.is_ascii_digit()) =>
        {
            if rest.is_empty() {
                "pad".to_string()
            } else {
                format!("pad {}", rest.join(" "))
            }
        }
        _ if !rest.is_empty()
            && first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && first.chars().any(|c| c.is_ascii_digit()) =>
        {
            // `<reference> keepout`, `<reference> copper fp_poly`.
            format!("footprint {}", rest.join(" "))
        }
        _ => label.to_string(),
    }
}

fn layers_text(mask: pcb_router::LayerMask, count: usize) -> String {
    (0..count)
        .map(|layer| if mask & (1 << layer) != 0 { '1' } else { '.' })
        .collect()
}

#[test]
fn the_lowering_of_f1_is_the_golden_table() {
    // Review 2026-10-21, fixture test 7 (and the lowering's half of test
    // 5): what F1's items lower to, by kind, the obstacles that are not
    // pads in full, the terminals of every net (the stacked pair two, the
    // jumper group one, net "14", the exposed pad's thermal vias merged)
    // and the pads closed to vias. A change of the lowering shows here as
    // a diff to acknowledge.
    let config = f1_config();
    let board = f1_board(&config);
    let layers = board.layer_count;
    let mut text = String::new();
    text.push_str(&format!(
        "layers {} classes {} nets {} obstacles {} planes {}\n",
        layers,
        board.classes.len(),
        board.nets.len(),
        board.obstacles.len(),
        board.planes.len()
    ));
    text.push_str(&format!(
        "edge_clearance {:.4} hole_clearance {:.4} hole_to_hole {:.4} neck_width {:.4}\n",
        board.edge_clearance, board.hole_clearance, board.hole_to_hole, board.neck_width
    ));
    for (index, class) in board.classes.iter().enumerate() {
        text.push_str(&format!(
            "class {index}: width {:.4} clearance {:.4} via {:.4}/{:.4}\n",
            class.trace_width, class.clearance, class.via_diameter, class.via_drill
        ));
    }
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for obstacle in &board.obstacles {
        *counts
            .entry((
                format!("{:?}", obstacle.kind),
                obstacle_category(&obstacle.label),
            ))
            .or_default() += 1;
    }
    text.push_str("\nobstacles by kind\n");
    for ((kind, category), count) in &counts {
        text.push_str(&format!("{kind:8} {count:3}  {category}\n"));
    }
    text.push_str("\nobstacles other than pads\n");
    let net_name = |net: Option<pcb_router::NetId>| {
        net.map_or("-".to_string(), |net| board.nets[net as usize].name.clone())
    };
    for obstacle in &board.obstacles {
        if obstacle_category(&obstacle.label) == "pad" {
            continue;
        }
        let bounds = obstacle.shape.aabb();
        text.push_str(&format!(
            "{:?} {} [{}] net {} clearance {:.4} tracks {} vias {} box ({:.3} {:.3})-({:.3} {:.3})\n",
            obstacle.kind,
            obstacle.label,
            layers_text(obstacle.layers, layers),
            net_name(obstacle.net),
            obstacle.clearance,
            obstacle.blocks_tracks,
            obstacle.blocks_vias,
            bounds.minimum[0],
            bounds.minimum[1],
            bounds.maximum[0],
            bounds.maximum[1],
        ));
    }
    text.push_str("\nterminals\n");
    for net in &board.nets {
        let terminals: Vec<String> = net
            .terminals
            .iter()
            .map(|terminal| {
                format!(
                    "{}[{}]",
                    terminal.label,
                    layers_text(terminal.layers, layers)
                )
            })
            .collect();
        text.push_str(&format!(
            "{} (class {}): {}\n",
            net.name,
            net.class,
            terminals.join(" ")
        ));
    }
    text.push_str("\nmerged pads\n");
    for merged in &board.merged_pads {
        text.push_str(&format!(
            "{} into {} of {}\n",
            board.obstacles[merged.pad].label,
            board.nets[merged.net as usize].terminals[merged.host].label,
            board.nets[merged.net as usize].name
        ));
    }
    text.push_str("\npads closed to vias\n");
    let closed: Vec<&str> = board
        .no_via_pads
        .iter()
        .map(|index| board.obstacles[*index].label.as_str())
        .collect();
    text.push_str(&format!("{}\n", closed.join(" ")));
    text.push_str("\nplanes\n");
    for plane in &board.planes {
        text.push_str(&format!(
            "{} layer {} class {} corners {} excluded {} above {} hatch {}\n",
            board.nets[plane.net as usize].name,
            plane.layer,
            plane.class,
            plane.polygon.len(),
            plane.excluded.len(),
            plane.above.len(),
            plane.hatch.is_some()
        ));
    }
    text.push_str(&format!(
        "\nsolid pads {:?}\nisolated pads {:?}\n",
        board.solid_pads, board.isolated_pads
    ));
    golden("f1-lowering.txt", &text);
    // What the table must say whatever the golden holds.
    assert!(
        !closed.contains(&"U1.17"),
        "the exposed pad keeps its thermal vias"
    );
    for pad in [
        "U1.1", "U2.2", "D1.1", "C1.1", "R1.2", "NT1.1", "JP1.1", "TP2.1",
    ] {
        assert!(closed.contains(&pad), "{pad} is closed to vias: {closed:?}");
    }
    let net = |name: &str| board.nets.iter().find(|net| net.name == name).unwrap();
    assert_eq!(
        net("SIG").terminals.len(),
        3,
        "the stacked pair is two items"
    );
    assert_eq!(net("14").terminals.len(), 3, "the jumper group is one item");
    assert_eq!(board.isolated_pads, ["R1.2"]);
    let via_in_pad = f1_board(&KiCadBoardRouterConfig {
        via_in_pad: Some(true),
        ..config
    });
    assert!(via_in_pad.no_via_pads.is_empty());
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
    // Review 2026-10-21, fixture test 9: the order test without the race.
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
        placement_seeds: 2,
        fixed: vec!["J1".into(), "H1".into()],
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

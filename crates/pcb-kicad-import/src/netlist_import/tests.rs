// Copyright (C) 2026 Toit contributors.

use super::*;

#[test]
fn missing_parts_fold_by_their_footprint() {
    // Trial 6's eurorack-pmod: two holes without a footprint and eight
    // diodes of one library the fp-lib-table names on the designer's
    // machine, one line per part before.
    let why = "(LLP1006-2M:VISHAY-LLP1006-2M not found: the fp-lib-table names \"LLP1006-2M\" at /home/seb/libs/LLP1006-2M.pretty, which is not there)";
    let mut lines = vec![
        "H1 (no footprint)".to_string(),
        "H2 (no footprint)".to_string(),
    ];
    lines.extend(
        ["D27", "D29", "D28"]
            .iter()
            .map(|reference| format!("{reference} {why}")),
    );
    lines.push("J3 (ffc:FH12 unreadable: no footprint form)".to_string());
    assert_eq!(
        fold_missing(&lines),
        vec![
            "2 parts: H1, H2 (no footprint)".to_string(),
            format!("3 parts: D27, D29, D28 {why}"),
            "J3 (ffc:FH12 unreadable: no footprint form)".to_string(),
        ]
    );
}

#[test]
fn an_import_folds_the_missing_parts_of_one_footprint() {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-fold-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "D27") (value "ESD") (footprint "LLP1006-2M:VISHAY-LLP1006-2M") (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
            (comp (ref "H1") (value "hole") (sheetpath (names "/") (tstamps "/")) (tstamps "2222"))
            (comp (ref "D28") (value "ESD") (footprint "LLP1006-2M:VISHAY-LLP1006-2M") (sheetpath (names "/") (tstamps "/")) (tstamps "3333")))
          (nets
            (net (code "1") (name "A") (node (ref "D27") (pin "1") (pintype "passive")) (node (ref "D28") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let report = import_kicad_netlist(&netlist, &directory.join("out"), "test", 2).unwrap();
    fs::remove_dir_all(&directory).ok();
    assert_eq!(report.missing_parts, 3);
    assert_eq!(report.missing.len(), 2, "{:?}", report.missing);
    assert!(
        report.missing[0].starts_with(
            "2 parts: D27, D28 (LLP1006-2M:VISHAY-LLP1006-2M not found: no library \"LLP1006-2M\""
        ),
        "{:?}",
        report.missing
    );
    assert_eq!(report.missing[1], "H1 (no footprint)");
}

#[test]
fn near_decoupling_puts_the_templates_near_entries_in_the_new_constraints() {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-near-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let library = directory.join("parts.pretty");
    fs::create_dir_all(&library).unwrap();
    let footprint = |name: &str, pads: &[(&str, f64)]| {
        let pads: String = pads
            .iter()
            .map(|(pad, x)| format!(r#"(pad "{pad}" smd rect (at {x} 0) (size 0.6 0.6) (layers "F.Cu" "F.Paste" "F.Mask"))"#))
            .collect();
        format!(
            r#"(footprint "{name}" (version 20260206) (generator "test") (layer "F.Cu")
            (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (property "Value" "{name}" (at 0 1.5 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
            (fp_rect (start -2 -1) (end 2 1) (layer "F.CrtYd"))
            (attr smd) {pads})"#
        )
    };
    fs::write(
        library.join("IC.kicad_mod"),
        footprint("IC", &[("1", -1.0), ("2", 0.0), ("3", 1.0)]),
    )
    .unwrap();
    fs::write(
        library.join("C.kicad_mod"),
        footprint("C", &[("1", -0.5), ("2", 0.5)]),
    )
    .unwrap();
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "U1") (value "MCU") (footprint "parts:IC") (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
            (comp (ref "C1") (value "100n") (footprint "parts:C") (sheetpath (names "/") (tstamps "/")) (tstamps "2222")))
          (nets
            (net (code "1") (name "+3V3") (node (ref "U1") (pin "1") (pintype "power_in")) (node (ref "C1") (pin "1") (pintype "passive")))
            (net (code "2") (name "GND") (node (ref "U1") (pin "2") (pintype "power_in")) (node (ref "C1") (pin "2") (pintype "passive")))
            (net (code "3") (name "A") (node (ref "U1") (pin "3") (pintype "output")))))"#,
    )
    .unwrap();
    // Off by default: the starter as before.
    let off = directory.join("off");
    let report =
        import_kicad_netlist_options(&netlist, &off, "test", Some(2), None, false).unwrap();
    assert_eq!(report.near_entries, None);
    let constraints: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(off.join("constraints.json")).unwrap()).unwrap();
    assert!(constraints.get("near").is_none(), "{constraints}");
    let on = directory.join("on");
    let report = import_kicad_netlist_options(&netlist, &on, "test", Some(2), None, true).unwrap();
    let constraints: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(on.join("constraints.json")).unwrap()).unwrap();
    fs::remove_dir_all(&directory).ok();
    assert_eq!(report.near_entries, Some(1));
    assert_eq!(
        constraints["near"],
        serde_json::json!([{"part": "C1", "pin_of": "U1:1", "max_mm": 1.5}])
    );
    assert_eq!(constraints["move_all"], true);
    assert!(
        constraints["comment"][0]
            .as_str()
            .unwrap()
            .starts_with("near:")
    );
}

#[test]
fn no_matrix_with_parts_missing_says_the_parts_are_missing() {
    let why = no_matrix_why(4, 88);
    assert!(why.contains("88 parts are missing"), "{why}");
    assert!(!why.contains("or no keyboard"), "{why}");
    assert!(no_matrix_why(4, 0).ends_with("(keys wired directly to pins, or no keyboard)"));
}

#[test]
fn stock_libraries_are_found_on_macos_and_by_kicads_variables() {
    let environment = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    };
    // KiCad's own variable first, on any platform; the macOS bundle
    // among the standard places.
    let directories = stock_footprint_directories(&environment(&[(
        "KICAD10_FOOTPRINT_DIR",
        "/opt/kicad/footprints",
    )]));
    assert_eq!(directories[0], Path::new("/opt/kicad/footprints"));
    assert!(directories.contains(&PathBuf::from(
        "/Applications/KiCad/KiCad.app/Contents/SharedSupport/footprints"
    )));
    let directories =
        stock_footprint_directories(&environment(&[("KICAD9_FOOTPRINT_DIR", "/opt/kicad9")]));
    assert_eq!(directories[0], Path::new("/opt/kicad9"));
    // The user's table on macOS, KiCad 10's before 9's.
    let tables = user_footprint_tables(&environment(&[("HOME", "/Users/a")]));
    let mac = PathBuf::from("/Users/a/Library/Preferences/kicad/10.0/fp-lib-table");
    let linux = PathBuf::from("/Users/a/.config/kicad/10.0/fp-lib-table");
    let older = PathBuf::from("/Users/a/Library/Preferences/kicad/9.0/fp-lib-table");
    let position = |table: &PathBuf| tables.iter().position(|known| known == table).unwrap();
    assert!(position(&linux) < position(&older) && position(&mac) < position(&older));
    let tables = user_footprint_tables(&environment(&[("APPDATA", "C:/Users/a/AppData/Roaming")]));
    assert_eq!(
        tables[0],
        Path::new("C:/Users/a/AppData/Roaming/kicad/10.0/fp-lib-table")
    );
}

#[test]
fn kicad_10_names_and_add_on_nicknames_are_found() {
    // The schematic trial's three: two WSON-8 that KiCad 10 renamed with
    // a vendor prefix, one ESP32 module of Espressif's add-on library.
    let dir = std::env::temp_dir().join(format!("ux3-kicad10-names-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let son = dir.join("Package_SON.pretty");
    let espressif = dir.join("3rdparty/footprints/com_github_espressif/PCM_Espressif.pretty");
    fs::create_dir_all(&son).unwrap();
    fs::create_dir_all(&espressif).unwrap();
    for name in [
        "Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm",
        "Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias",
        "A_SOIC-8",
        "B_SOIC-8",
    ] {
        fs::write(son.join(format!("{name}.kicad_mod")), "(footprint)").unwrap();
    }
    fs::write(espressif.join("ESP32-S3-MINI-1.kicad_mod"), "(footprint)").unwrap();
    let libraries = BTreeMap::from([
        ("Package_SON".to_string(), son.clone()),
        ("PCM_Espressif".to_string(), espressif.clone()),
    ]);
    let found = |id: &str| find_footprint(&libraries, id).map(|(_, found)| found);
    let wson = "Package_SON:WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias";
    let renamed = "Package_SON:Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm_ThermalVias";
    assert_eq!(found(wson).as_deref(), Some(renamed));
    assert_eq!(
        found("Package_SON:WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm").as_deref(),
        Some("Package_SON:Texas_DSG0008A_WSON-8-1EP_2x2mm_P0.5mm_EP0.9x1.6mm")
    );
    assert!(substitution_reason(wson, renamed).contains("KiCad 10's name"));
    // Two footprints end in the name: neither is taken.
    assert_eq!(found("Package_SON:SOIC-8"), None);
    // The add-on under its Plugin and Content Manager nickname.
    let esp = found("Espressif:ESP32-S3-MINI-1").unwrap();
    assert_eq!(esp, "PCM_Espressif:ESP32-S3-MINI-1");
    assert!(substitution_reason("Espressif:ESP32-S3-MINI-1", &esp).contains("add-on"));
    // KiCad's add-on folder, by its variable and by default.
    let environment = |name: &str| match name {
        "KICAD10_3RD_PARTY" => Some("/opt/3rdparty".to_string()),
        "HOME" => Some("/home/a".to_string()),
        _ => None,
    };
    let folders = third_party_directories(&environment);
    assert_eq!(folders[0], Path::new("/opt/3rdparty"));
    assert!(folders.contains(&PathBuf::from("/home/a/.local/share/kicad/10.0/3rdparty")));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn plug_in_connectors_are_told_by_their_footprint_names() {
    for name in [
        "Connector_USB:USB_C_Receptacle_GCT_USB4105",
        "x:USB2.0_TYPE-C",
        "Connector_Coaxial:SMA_Amphenol_901-144_Vertical",
        "Connector_BarrelJack:BarrelJack_Horizontal",
        "Connector_Audio:Jack_3.5mm_CUI_SJ-3523-SMT_Horizontal",
        "Connector_RJ:RJ45_Amphenol_54602-x08_Horizontal",
        "Connector_Dsub:DSUB-9_Socket_Horizontal",
        "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-2",
        "Main:PCIEXPRESS-X1",
    ] {
        assert!(edge_connector(name), "{name}");
    }
    for name in [
        "Logos:LordsBoardsLogo_Small_Silk",
        "Connector_Coaxial:U.FL_Hirose_U.FL-R-SMT-1_Vertical",
        "Connector_PinHeader_2.54mm:PinHeader_1x04_P2.54mm_Vertical",
        "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
        "Diode_SMD:D_SMA",
        "D_SMA_Handsoldering",
        "Diode_SMD:D_SMA-SMB_Universal_Handsoldering",
    ] {
        assert!(!edge_connector(name), "{name}");
    }
}

#[test]
fn kicad_5_arcs_become_three_points() {
    // A quarter circle around (2, 0) from (2, 1) to (3, 0), as the
    // Tag-Connect footprint of osprey's library draws its corners.
    let mut footprint = parse(r#"(footprint "x" (fp_arc (start 2 0) (end 2 1) (angle -90) (layer "F.SilkS") (width 0.2)) (fp_arc (start 0 0) (mid 1 1) (end 2 0) (layer "F.SilkS")))"#).unwrap();
    let Expr::List(items) = &mut footprint else {
        unreachable!()
    };
    legacy_arcs_to_three_points(items).unwrap();
    let arc = &footprint.children()[2];
    assert_eq!(encode(arc), encode(&parse(r#"(fp_arc (start 2 1) (mid 2.707107 0.707107) (end 3 0) (layer "F.SilkS") (width 0.2))"#).unwrap()));
    // A KiCad 7 arc stays as it is.
    assert_eq!(
        encode(&footprint.children()[3]),
        encode(&parse(r#"(fp_arc (start 0 0) (mid 1 1) (end 2 0) (layer "F.SilkS"))"#).unwrap())
    );
}

#[test]
fn legacy_text_conversion_keeps_the_lock_and_the_actual_text() {
    let mut forms = [
        parse(r#"(fp_text reference locked "C2" (at 0 0) (layer "B.SilkS"))"#).unwrap(),
        parse(r#"(fp_text value "locked" (at 0 1) (layer "B.Fab"))"#).unwrap(),
    ];
    legacy_texts_to_properties(&mut forms, "C2").unwrap();
    assert_eq!(forms[0].children()[2].atom(), Some("C2"));
    assert_eq!(form_atom(&forms[0], "locked", 1), Some("yes"));
    assert_eq!(forms[1].children()[2].atom(), Some("locked"));
    assert!(forms[1].child("locked").is_none());
}

#[test]
fn kicad_5_library_footprints_become_footprints_of_today() {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-import-k5-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let library = directory.join("old.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::write(
        library.join("FB.kicad_mod"),
        r#"(module FB (layer F.Cu) (tedit 5A0F774F)
            (fp_text reference REF** (at 0 -2) (layer F.SilkS) (effects (font (size 1 1) (thickness 0.15))))
            (fp_text value FB (at 0 2) (layer F.Fab) (effects (font (size 1 1) (thickness 0.15))))
            (fp_arc (start 0 0) (end 0 1) (angle -90) (layer F.SilkS) (width 0.12))
            (pad 1 thru_hole circle (at -2 0) (size 1.6 1.6) (drill 0.8) (layers *.Cu *.Mask))
            (pad 2 thru_hole circle (at 2 0) (size 1.6 1.6) (drill 0.8) (layers *.Cu *.Mask)))"#,
    )
    .unwrap();
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components (comp (ref "FB1") (value "FB") (footprint "old:FB") (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
          (nets (net (code "1") (name "VCC") (node (ref "FB1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let report = import_kicad_netlist(&netlist, &directory.join("out"), "test", 2).unwrap();
    assert!(report.missing.is_empty(), "{:?}", report.missing);
    let board = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
    let footprint = board
        .children()
        .iter()
        .find(|item| item.head() == Some("footprint"))
        .expect("a footprint, not a module");
    let arc = footprint.child("fp_arc").unwrap();
    assert!(arc.child("mid").is_some() && arc.child("angle").is_none());
    assert_eq!(
        form_atom(footprint.child("pad").unwrap(), "net", 1),
        Some("VCC")
    );
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_table_entry_at_another_machines_path_is_found_by_its_last_folders() {
    // Fresh-agent trial 6, eurorack-pmod: the fp-lib-table names
    // AudioJacksFixed at /home/seb/dev/eurorack-pmod/hardware/libs/custom/
    // AudioJacksFixed.pretty, the checkout has it at ../libs/custom; the
    // import said "no library in the fp-lib-table files". LLP1006-2M is
    // in the table and nowhere: the message names the table's folder.
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-relocated-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let project = directory.join("hardware/board-r3");
    let library = directory.join("hardware/libs/custom/Jacks.pretty");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&library).unwrap();
    fs::write(
        project.join("fp-lib-table"),
        r#"(fp_lib_table (version 7)
          (lib (name "Jacks") (type "KiCad") (uri "/nowhere/seb/dev/x/hardware/libs/custom/Jacks.pretty") (options "") (descr ""))
          (lib (name "Gone") (type "KiCad") (uri "/nowhere/seb/dev/x/hardware/libs/custom/Gone.pretty") (options "") (descr "")))"#,
    )
    .unwrap();
    fs::write(
        library.join("J_Test.kicad_mod"),
        r#"(footprint "J_Test" (version 20260206) (generator "test") (layer "F.Cu")
            (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (property "Value" "J_Test" (at 0 1.5 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
            (attr smd)
            (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask"))
            (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask")))"#,
    )
    .unwrap();
    let netlist = project.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "J1") (value "jack") (footprint "Jacks:J_Test") (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
            (comp (ref "D1") (value "esd") (footprint "Gone:D_Test") (sheetpath (names "/") (tstamps "/")) (tstamps "2222")))
          (nets
            (net (code "1") (name "IN") (node (ref "J1") (pin "1") (pintype "passive")) (node (ref "D1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let output = directory.join("out");
    let report = import_kicad_netlist(&netlist, &output, "test", 2).unwrap();
    assert_eq!(report.footprints, 1);
    assert_eq!(
        report.relocated_libraries.len(),
        1,
        "{:?}",
        report.relocated_libraries
    );
    assert!(
        report.relocated_libraries[0].starts_with(
            "Jacks: the fp-lib-table's /nowhere/seb/dev/x/hardware/libs/custom/Jacks.pretty is not here; used "
        ) && report.relocated_libraries[0].ends_with("hardware/libs/custom/Jacks.pretty"),
        "{:?}",
        report.relocated_libraries
    );
    assert_eq!(report.missing.len(), 1);
    assert!(
        report.missing[0].starts_with(
            "D1 (Gone:D_Test not found: the fp-lib-table names \"Gone\" at /nowhere/seb/dev/x/hardware/libs/custom/Gone.pretty, which is not there"
        ),
        "{:?}",
        report.missing
    );
    // Put where the message says (the output directory), the library is
    // found by the next import.
    let gone = output.join("Gone.pretty");
    fs::create_dir_all(&gone).unwrap();
    fs::write(
        gone.join("D_Test.kicad_mod"),
        fs::read_to_string(library.join("J_Test.kicad_mod"))
            .unwrap()
            .replace("J_Test", "D_Test"),
    )
    .unwrap();
    let again = import_kicad_netlist(&netlist, &output, "test", 2).unwrap();
    assert!(again.missing.is_empty(), "{:?}", again.missing);
    assert_eq!(again.footprints, 2);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn netlist_becomes_a_board_with_nets_fields_and_symbol_links() {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-import-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let library = directory.join("libs/parts.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::write(
        directory.join("fp-lib-table"),
        r#"(fp_lib_table (version 7) (lib (name "parts") (type "KiCad") (uri "$(KIPRJMOD)/libs/parts.pretty") (options "") (descr "")))"#,
    )
    .unwrap();
    fs::write(
        library.join("R_Test.kicad_mod"),
        r#"(footprint "R_Test" (version 20260206) (generator "test") (layer "F.Cu")
            (property "Reference" "REF**" (at 0 -1.5 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (property "Value" "R_Test" (at 0 1.5 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
            (attr smd)
            (pad "1" smd rect (at -1 0) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask"))
            (pad "2" smd rect (at 1 0) (size 1 1) (layers "F.Cu" "F.Paste" "F.Mask")))"#,
    )
    .unwrap();
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "R1") (value "10k") (footprint "parts:R_Test")
              (fields (field (name "Footprint") "parts:R_Test") (field (name "Datasheet")) (field (name "LCSC") "C25744"))
              (property (name "Sheetfile") (value "test.kicad_sch"))
              (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
            (comp (ref "R2") (value "1k") (footprint "Missing:R_Nowhere")
              (property (name "dnp")) (sheetpath (names "/") (tstamps "/")) (tstamps "2222"))
            (comp (ref "R3") (value "1k") (footprint "other:R_Test")
              (property (name "dnp")) (sheetpath (names "/sub/") (tstamps "/abcd/")) (tstamps "3333")))
          (nets
            (net (code "1") (name "/VIN") (node (ref "R1") (pin "1") (pintype "passive")))
            (net (code "2") (name "GND") (node (ref "R1") (pin "2") (pintype "passive")) (node (ref "R3") (pin "2") (pintype "passive")))))"#,
    )
    .unwrap();
    let output = directory.join("out");
    let report = import_kicad_netlist(&netlist, &output, "test", 4).unwrap();
    assert_eq!(report.footprints, 2);
    assert_eq!(report.nets, 2);
    assert_eq!(report.missing, vec!["R2 (Missing:R_Nowhere not found: no library \"Missing\" in the fp-lib-table files, the schematic folder's or the output directory's .pretty folders, KiCad's own or its add-on libraries)".to_string()]);
    assert_eq!(
        report.substituted,
        vec!["R3: other:R_Test -> parts:R_Test".to_string()]
    );
    let [changed] = report.footprint_id_changes.as_slice() else {
        panic!("expected one emitted footprint with a different ID");
    };
    assert_eq!(changed.part, "R3");
    assert_eq!(changed.source_footprint.as_deref(), Some("other:R_Test"));
    assert_eq!(changed.imported_footprint, "parts:R_Test");

    let board = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
    let layers = board.child("layers").unwrap().children();
    assert!(
        layers
            .iter()
            .any(|layer| layer.children().get(1).and_then(Expr::atom) == Some("In2.Cu"))
    );
    let footprints: Vec<&Expr> = board
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    let property = |footprint: &Expr, name: &str| {
        footprint
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("property")
                    && item.children().get(1).and_then(Expr::atom) == Some(name)
            })
            .and_then(|item| item.children().get(2).and_then(Expr::atom))
            .map(str::to_owned)
    };
    let r1 = footprints[0];
    assert_eq!(r1.children()[1].atom(), Some("parts:R_Test"));
    assert_eq!(property(r1, "Reference").as_deref(), Some("R1"));
    assert_eq!(property(r1, "Value").as_deref(), Some("10k"));
    assert_eq!(property(r1, "LCSC").as_deref(), Some("C25744"));
    assert_eq!(form_atom(r1, "path", 1), Some("/1111"));
    assert_eq!(form_atom(r1, "sheetfile", 1), Some("test.kicad_sch"));
    assert!(
        r1.child("version").is_none() && r1.child("at").is_some() && r1.child("uuid").is_some()
    );
    let nets: Vec<Option<&str>> = r1
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .map(|pad| form_atom(pad, "net", 1))
        .collect();
    assert_eq!(nets, vec![Some("/VIN"), Some("GND")]);
    let r3 = footprints[1];
    assert_eq!(form_atom(r3, "path", 1), Some("/abcd/3333"));
    assert_eq!(form_atom(r3, "sheetname", 1), Some("/sub/"));
    assert!(
        r3.child("attr")
            .unwrap()
            .children()
            .iter()
            .any(|flag| flag.atom() == Some("dnp"))
    );
    let pads: Vec<Option<&str>> = r3
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .map(|pad| form_atom(pad, "net", 1))
        .collect();
    assert_eq!(pads, vec![None, Some("GND")]);
    // A second import keeps constraints.json and layout.json and still
    // names them.
    assert!(report.kept.is_empty() && report.constraints.is_some());
    let again = import_kicad_netlist(&netlist, &output, "test", 4).unwrap();
    assert_eq!(again.constraints, report.constraints);
    assert_eq!(again.layout, report.layout);
    assert_eq!(again.kept, vec!["constraints.json", "layout.json"]);
    fs::remove_dir_all(&directory).unwrap();
}
#[test]
fn the_footprints_board_gives_the_copper_layers_unless_layers_does() {
    // Fresh-agent trial 6: eurorack-pmod's designer's board has six
    // copper layers, the import made two and the report did not say.
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-import-layers-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let board = directory.join("designer.kicad_pcb");
    fs::write(
        &board,
        r#"(kicad_pcb (version 20221018) (generator pcbnew)
          (layers (0 "F.Cu" signal) (4 "In1.Cu" signal) (6 "In2.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user))
          (footprint "personal:SOT_Mine" (layer "F.Cu") (tstamp 11111111-0000-4000-8000-000000000002) (at 30 20 180)
            (path "/2222")
            (fp_text reference "U1" (at 0 -2 180) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" smd rect (at -1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 2 "GND"))
            (pad "2" smd rect (at 1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 1 "/VIN"))))"#,
    )
    .unwrap();
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "U1") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "2222")))
          (nets
            (net (code "1") (name "/VIN") (node (ref "U1") (pin "2") (pintype "passive")))
            (net (code "2") (name "GND") (node (ref "U1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let inner = |report: &KiCadNetlistImport| {
        parse(&fs::read_to_string(&report.board).unwrap())
            .unwrap()
            .child("layers")
            .unwrap()
            .children()
            .iter()
            .filter(|layer| {
                layer
                    .children()
                    .get(1)
                    .and_then(Expr::atom)
                    .is_some_and(|name| name.starts_with("In"))
            })
            .count()
    };
    let report =
        import_kicad_netlist_with(&netlist, &directory.join("a"), "test", None, Some(&board))
            .unwrap();
    assert_eq!(report.copper_layers, 4);
    assert_eq!(
        report.copper_layers_from,
        "the --footprints-from board's (--layers sets another)"
    );
    assert_eq!(inner(&report), 2);
    let report = import_kicad_netlist_with(
        &netlist,
        &directory.join("b"),
        "test",
        Some(2),
        Some(&board),
    )
    .unwrap();
    assert_eq!(report.copper_layers, 2);
    assert_eq!(
        report.copper_layers_from,
        "--layers (the --footprints-from board has 4)"
    );
    assert_eq!(inner(&report), 0);
    let report =
        import_kicad_netlist_with(&netlist, &directory.join("c"), "test", None, None).unwrap();
    assert_eq!(report.copper_layers, 2);
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_missing_legacy_library_keeps_the_donor_id_only_in_a_safe_output_table() {
    let root = temporary_next_to(&std::env::temp_dir().join("legacy-donor"));
    let source = root.join("source");
    fs::create_dir_all(&source).unwrap();
    let netlist = source.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E") (components (comp (ref "U1") (value "Old")
            (footprint "Archive:Chip") (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))) (nets))"#,
    )
    .unwrap();
    let donor = root.join("donor.kicad_pcb");
    fs::write(
        &donor,
        r#"(kicad_pcb (version 20221018) (generator pcbnew)
            (footprint "Archive:Chip" (layer "F.Cu") (at 10 20) (path "/1111")
                (fp_text reference "U1" (at 0 1) (layer "F.SilkS"))
                (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu" "F.Mask"))))"#,
    )
    .unwrap();
    let table = source.join("fp-lib-table");
    let row = |uri: &str| {
        format!(
            r#"(lib (name "Archive") (type "Legacy") (uri "{uri}") (options "") (descr "original"))"#
        )
    };
    let unresolved = row("${PCB_MAKER_TEST_UNKNOWN_LIBRARY_PATH}/Archive.mod");
    let missing = row("${KIPRJMOD}/missing/Archive.mod");
    let external = root.join("external.mod");
    fs::write(&external, "external legacy contents").unwrap();
    let external_row = row(external.to_str().unwrap());
    let unrelated = r#"(lib (name "Other") (type "KiCad") (uri "${KIPRJMOD}/Other.pretty") (options "") (descr "user-owned"))"#;
    for (name, source_rows, output_rows, keep_id) in [
        ("missing", missing.clone(), missing.clone(), true),
        (
            "external",
            external_row.clone(),
            external_row.clone(),
            false,
        ),
        ("unresolved", unresolved.clone(), unresolved, false),
        ("conflict", missing.clone(), external_row.clone(), false),
        ("duplicate", format!("{missing} {missing}"), missing, false),
    ] {
        let input_contents = format!("(fp_lib_table {source_rows})\n");
        fs::write(&table, &input_contents).unwrap();
        let output = root.join(name);
        let existing = output.join("Archive.pretty");
        fs::create_dir_all(&existing).unwrap();
        let sentinel = existing.join("Custom.kicad_mod");
        fs::write(&sentinel, "user's model").unwrap();
        let copied = output.join("fp-lib-table");
        fs::write(
            &copied,
            format!("(fp_lib_table {output_rows} {unrelated})\n"),
        )
        .unwrap();
        let report = import_kicad_netlist_with_options(
            &netlist,
            &output,
            "test",
            KiCadNetlistImportOptions {
                footprints_from: Some(&donor),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.missing.is_empty(), "{name}: {:?}", report.missing);
        let expected = if keep_id {
            "Archive:Chip"
        } else {
            "from_board:Chip"
        };
        let board =
            crate::board_footprints::BoardFootprints::read(&output.join("test.kicad_pcb")).unwrap();
        assert_eq!(board.footprints[0].id, expected, "{name}");
        let copied = fs::read_to_string(copied).unwrap();
        assert_eq!(copied.matches(r#"(name "Archive")"#).count(), 1, "{name}");
        assert!(copied.contains(r#"(descr "user-owned")"#));
        assert_eq!(copied.contains(r#"(type "Legacy")"#), !keep_id, "{name}");
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "user's model");
        assert_eq!(fs::read_to_string(&table).unwrap(), input_contents);
        let generated = if keep_id {
            "Archive-2.pretty"
        } else {
            "from_board.pretty"
        };
        assert!(
            output.join(generated).join("Chip.kicad_mod").is_file(),
            "{name}"
        );
        if keep_id {
            assert_eq!(
                fs::read_to_string(output.join(generated).join("Custom.kicad_mod")).unwrap(),
                "user's model"
            );
            assert!(copied.contains("${KIPRJMOD}/Archive-2.pretty"));
        }
    }
    assert_eq!(
        fs::read_to_string(external).unwrap(),
        "external legacy contents"
    );
    let contents = fs::read_to_string(&table).unwrap();
    let without_donor = root.join("without-donor");
    let report = import_kicad_netlist(&netlist, &without_donor, "test", 2).unwrap();
    assert_eq!(report.missing_parts, 1);
    assert!(report.missing[0].contains("Convert it to a KiCad .pretty library"));
    assert!(!without_donor.join("Archive.pretty").exists());
    let error = import_kicad_netlist_with_options(
        &netlist,
        &source,
        "test",
        KiCadNetlistImportOptions {
            footprints_from: Some(&donor),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("separate output directory"), "{error}");
    assert_eq!(fs::read_to_string(table).unwrap(), contents);
    assert!(!source.join("test.kicad_pcb").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn footprints_come_from_the_board_where_no_library_has_them_and_the_board_version_wins() {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-import-board-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // A library with an old, larger R_Test (drifted: 6 x 2 against 3 x 1).
    let library = directory.join("parts.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::write(
        library.join("R_Test.kicad_mod"),
        r#"(footprint "R_Test" (layer "F.Cu")
            (fp_rect (start -3 -1) (end 3 1) (layer "F.CrtYd") (stroke (width 0.05) (type solid)))
            (pad "1" smd rect (at -2 0) (size 1 1) (layers "F.Cu" "F.Mask"))
            (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu" "F.Mask")))"#,
    )
    .unwrap();
    // The designer's board: R1 (parts:R_Test, smaller, on the back,
    // turned), U1 from a personal library nobody has, H1 a mounting hole
    // the schematic lacks, plated on GND.
    let board = directory.join("designer.kicad_pcb");
    fs::write(
        &board,
        r#"(kicad_pcb (version 20221018) (generator pcbnew)
          (footprint "parts:R_Test" (layer "B.Cu") (tstamp 11111111-0000-4000-8000-000000000001) (at 10 20 90)
            (path "/1111")
            (fp_text reference "R1" (at 0 1.5 90) (layer "B.SilkS") (effects (font (size 1 1) (thickness 0.15)) (justify mirror)))
            (fp_rect (start -1.5 0.5) (end 1.5 -0.5) (layer "B.CrtYd") (stroke (width 0.05) (type solid)))
            (pad "1" smd rect (at -1 0 90) (size 0.6 0.6) (layers "B.Cu" "B.Mask") (net 1 "/VIN"))
            (pad "2" smd rect (at 1 0 90) (size 0.6 0.6) (layers "B.Cu" "B.Mask") (net 2 "GND")))
          (footprint "personal:SOT_Mine" (layer "F.Cu") (tstamp 11111111-0000-4000-8000-000000000002) (at 30 20 180)
            (path "/2222")
            (fp_text reference "U1" (at 0 -2 180) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" smd rect (at -1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 2 "GND"))
            (pad "2" smd rect (at 1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 1 "/VIN")))
          (footprint "MountingHole:Hole_Pad" (layer "F.Cu") (tstamp 11111111-0000-4000-8000-000000000003) (at 3 3)
            (fp_text reference "H1" (at 0 -3) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" thru_hole circle (at 0 0) (size 5 5) (drill 3) (layers "*.Cu" "*.Mask") (net 2 "GND"))))"#,
    )
    .unwrap();
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "R1") (value "10k") (footprint "parts:R_Test") (sheetpath (names "/") (tstamps "/")) (tstamps "1111"))
            (comp (ref "U1") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "2222")))
          (nets
            (net (code "1") (name "/VIN") (node (ref "R1") (pin "1") (pintype "passive")) (node (ref "U1") (pin "2") (pintype "passive")))
            (net (code "2") (name "GND") (node (ref "R1") (pin "2") (pintype "passive")) (node (ref "U1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let output = directory.join("out");
    // Without the board, U1 is missing.
    let report = import_kicad_netlist(&netlist, &output, "test", 2).unwrap();
    assert_eq!(report.missing.len(), 1, "{:?}", report.missing);
    assert!(report.footprints_from_board.is_none() && report.drifted_footprints.is_none());
    // With it, and with an `add` for the mounting hole (kept where the
    // board has it) and a library footprint placed by the layout.
    fs::write(
        output.join("constraints.json"),
        r#"{"version": 1, "add": [{"part": "H1", "from_board": true},
            {"part": "TP1", "footprint": "parts:R_Test", "nets": {"1": "GND"}, "x": 5, "y": 6}]}"#,
    )
    .unwrap();
    let report = import_kicad_netlist_from(&netlist, &output, "test", 2, Some(&board)).unwrap();
    assert!(report.missing.is_empty(), "{:?}", report.missing);
    assert_eq!(
        report.footprints_from_board.as_deref(),
        Some(&["U1: personal:SOT_Mine (the board's U1)".to_string()][..])
    );
    assert_eq!(report.board_versions, Some(1));
    let drifted = report.drifted_footprints.clone().unwrap();
    assert_eq!(drifted.len(), 1, "{drifted:?}");
    assert!(
        drifted[0].starts_with(
            "parts:R_Test (1 parts: R1): library 6.00 x 2.00 mm, 2 pads; board 3.00 x 1.00 mm"
        ),
        "{drifted:?}"
    );
    assert_eq!(report.added.len(), 2, "{:?}", report.added);
    assert!(
        report.added[0].starts_with(
            "H1: MountingHole:Hole_Pad, pads on nets (1 GND), where the board has it (3 3, locked)"
        ),
        "{:?}",
        report.added
    );
    // The personal library is in the project now, with its table entry.
    let written =
        parse(&fs::read_to_string(output.join("personal.pretty/SOT_Mine.kicad_mod")).unwrap())
            .unwrap();
    assert_eq!(written.children()[1].atom(), Some("SOT_Mine"));
    assert!(written.child("at").is_none() && written.child("path").is_none());
    let pad = written
        .children()
        .iter()
        .find(|item| item.head() == Some("pad"))
        .unwrap();
    assert_eq!(form_at(pad).unwrap(), [-1.0, 1.0, 0.0]);
    assert!(pad.child("net").is_none());
    assert!(
        fs::read_to_string(output.join("fp-lib-table"))
            .unwrap()
            .contains("${KIPRJMOD}/personal.pretty")
    );

    let imported = parse(&fs::read_to_string(&report.board).unwrap()).unwrap();
    let footprint = |reference: &str| {
        imported
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("footprint")
                    && footprint_reference(item).as_deref() == Some(reference)
            })
            .unwrap()
            .clone()
    };
    // R1 is the board's version, in library form on the front.
    let r1 = footprint("R1");
    assert_eq!(form_atom(&r1, "layer", 1), Some("F.Cu"));
    let pads: Vec<[f64; 3]> = r1
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .map(|pad| form_at(pad).unwrap())
        .collect();
    assert_eq!(pads, vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]]);
    assert_eq!(
        form_atom(
            r1.children()
                .iter()
                .find(|item| item.head() == Some("pad"))
                .unwrap(),
            "net",
            1
        ),
        Some("/VIN")
    );
    // H1 where the board has it, locked, board only, on GND.
    let h1 = footprint("H1");
    assert_eq!(form_at(&h1).unwrap(), [3.0, 3.0, 0.0]);
    assert!(h1.child("locked").is_some());
    assert!(
        h1.child("attr")
            .unwrap()
            .children()
            .iter()
            .any(|flag| flag.atom() == Some("board_only"))
    );
    assert_eq!(form_atom(h1.child("pad").unwrap(), "net", 1), Some("GND"));
    // TP1 from the library, its pad 1 on GND, pad 2 on none; the layout
    // puts it at 5, 6 from the outline's corner.
    let tp1 = footprint("TP1");
    let nets: Vec<Option<&str>> = tp1
        .children()
        .iter()
        .filter(|item| item.head() == Some("pad"))
        .map(|pad| form_atom(pad, "net", 1))
        .collect();
    assert_eq!(nets, vec![Some("GND"), None]);
    let mut constraints = read_placement_constraints(&output.join("constraints.json")).unwrap();
    crate::placement_constraints::expand_additions(&mut constraints, &imported).unwrap();
    assert!(constraints.add.is_empty());
    assert_eq!(constraints.place.len(), 1);
    assert_eq!(
        (
            constraints.place[0].part.as_str(),
            constraints.place[0].x,
            constraints.place[0].y
        ),
        ("TP1", Some(5.0), Some(6.0))
    );
    // The layout refuses an added part the board does not have.
    let mut stale = read_placement_constraints(&output.join("constraints.json")).unwrap();
    let empty = parse("(kicad_pcb)").unwrap();
    assert!(
        crate::placement_constraints::expand_additions(&mut stale, &empty)
            .unwrap_err()
            .contains("import again")
    );

    // An added part may not take a schematic part's reference, and
    // from_board needs the board.
    fs::write(
        output.join("constraints.json"),
        r#"{"version": 1, "add": [{"part": "R1", "footprint": "parts:R_Test"}]}"#,
    )
    .unwrap();
    assert!(
        import_kicad_netlist(&netlist, &output, "test", 2)
            .unwrap_err()
            .contains("has a part of that reference already")
    );
    fs::write(
        output.join("constraints.json"),
        r#"{"version": 1, "add": [{"part": "H1", "from_board": true}]}"#,
    )
    .unwrap();
    assert!(
        import_kicad_netlist(&netlist, &output, "test", 2)
            .unwrap_err()
            .contains("--footprints-from")
    );
    fs::remove_dir_all(&directory).unwrap();
}
#[test]
fn footprints_from_the_board_keep_their_ids_for_the_drc_exclusions() {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-netlist-import-ids-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // A KiCad 6 library footprint: some pads lack IDs, and `tstamp`s in
    // the library otherwise start out identical for each instance.
    let library = directory.join("parts.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::write(
        library.join("C_Old.kicad_mod"),
        r#"(footprint "C_Old" (layer "F.Cu") (tstamp 22222222-0000-4000-8000-000000000000)
            (fp_rect (start -1 -0.5) (end 1 0.5) (layer "F.CrtYd") (width 0.05) (tstamp 22222222-0000-4000-8000-000000000001))
            (pad "1" smd rect (at -0.5 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask") (tstamp 22222222-0000-4000-8000-000000000002))
            (pad "2" smd rect (at 0.5 0) (size 0.5 0.5) (layers "F.Cu" "F.Mask"))
            (pad "2" smd rect (at 0.5 1) (size 0.5 0.5) (layers "F.Cu" "F.Mask"))
            (pad "" np_thru_hole circle (at 0 2) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask")))"#,
    )
    .unwrap();
    // The designer's board: U1 (its own footprint, from a library
    // nobody has), H1 (a mounting hole on GND the schematic lacks).
    let board = directory.join("designer.kicad_pcb");
    fs::write(
        &board,
        r#"(kicad_pcb (version 20240108) (generator pcbnew)
          (footprint "personal:SOT_Mine" (layer "F.Cu") (uuid "11111111-0000-4000-8000-000000000002") (at 30 20 180)
            (path "/2222")
            (property "Reference" "U1" (at 0 -2 180) (layer "F.SilkS") (uuid "11111111-0000-4000-8000-0000000000a0") (effects (font (size 1 1) (thickness 0.15))))
            (fp_rect (start -1.5 -1.5) (end 1.5 1.5) (layer "F.CrtYd") (stroke (width 0.05) (type solid)) (uuid "11111111-0000-4000-8000-0000000000a1"))
            (pad "1" smd rect (at -1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 2 "GND") (uuid "11111111-0000-4000-8000-0000000000a2"))
            (pad "2" smd rect (at 1 1 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 1 "/VIN") (uuid "11111111-0000-4000-8000-0000000000a3"))
            (pad "2" smd rect (at 1 2 180) (size 0.6 0.8) (layers "F.Cu" "F.Mask") (net 1 "/VIN")))
          (footprint "MountingHole:Hole_Pad" (layer "F.Cu") (uuid "11111111-0000-4000-8000-000000000003") (at 3 3)
            (property "Reference" "H1" (at 0 -3 0) (layer "F.SilkS") (uuid "11111111-0000-4000-8000-0000000000b0") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" thru_hole circle (at 0 0) (size 5 5) (drill 3) (layers "*.Cu" "*.Mask") (net 2 "GND") (uuid "11111111-0000-4000-8000-0000000000b1"))
            (pad "" np_thru_hole circle (at 0 3) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))))"#,
    )
    .unwrap();
    // U2 has U1's footprint, which the board has only as U1's: a copy.
    let netlist = directory.join("test.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "U1") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "2222"))
            (comp (ref "U2") (value "X") (footprint "personal:SOT_Mine") (sheetpath (names "/") (tstamps "/")) (tstamps "3333"))
            (comp (ref "C1") (value "1u") (footprint "parts:C_Old") (sheetpath (names "/") (tstamps "/")) (tstamps "4444"))
            (comp (ref "C2") (value "1u") (footprint "parts:C_Old") (sheetpath (names "/") (tstamps "/")) (tstamps "5555")))
          (nets
            (net (code "1") (name "/VIN") (node (ref "U1") (pin "2") (pintype "passive")) (node (ref "U2") (pin "2") (pintype "passive")))
            (net (code "2") (name "GND") (node (ref "U1") (pin "1") (pintype "passive")) (node (ref "C1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let output = directory.join("out");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        output.join("constraints.json"),
        r#"{"version": 1, "add": [{"part": "H1", "from_board": true}]}"#,
    )
    .unwrap();
    let report = import_kicad_netlist_from(&netlist, &output, "test", 2, Some(&board)).unwrap();
    assert!(report.missing.is_empty(), "{:?}", report.missing);
    let initial = fs::read_to_string(&report.board).unwrap();
    let imported = parse(&initial).unwrap();
    let other = directory.join("out-again");
    fs::create_dir_all(&other).unwrap();
    fs::copy(
        output.join("constraints.json"),
        other.join("constraints.json"),
    )
    .unwrap();
    let repeated = import_kicad_netlist_from(&netlist, &other, "test", 2, Some(&board)).unwrap();
    assert_eq!(initial, fs::read_to_string(&repeated.board).unwrap());
    for footprint in imported
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|item| item.head() == Some("pad"))
        {
            assert!(
                form_atom(pad, "uuid", 1)
                    .or_else(|| form_atom(pad, "tstamp", 1))
                    .is_some(),
                "pad in {:?} lacks an ID: {pad:?}",
                footprint_reference(footprint)
            );
        }
    }
    let ids = |reference: &str| -> Vec<String> {
        fn collect(item: &Expr, into: &mut Vec<String>) {
            if matches!(item.head(), Some("uuid" | "tstamp")) {
                into.push(
                    item.children()[1]
                        .atom()
                        .unwrap()
                        .trim_matches('"')
                        .to_string(),
                );
            }
            for child in item.children() {
                collect(child, into);
            }
        }
        let footprint = imported
            .children()
            .iter()
            .find(|item| {
                item.head() == Some("footprint")
                    && footprint_reference(item).as_deref() == Some(reference)
            })
            .unwrap();
        let mut ids = Vec::new();
        collect(footprint, &mut ids);
        ids
    };
    // U1 and H1 keep the board's ids: the footprint's, its pads' and
    // drawings' (the project's DRC exclusions name them).
    let u1 = ids("U1");
    assert_eq!(u1[0], "11111111-0000-4000-8000-000000000002", "{u1:?}");
    for own in [
        "11111111-0000-4000-8000-0000000000a1",
        "11111111-0000-4000-8000-0000000000a2",
        "11111111-0000-4000-8000-0000000000a3",
    ] {
        assert!(u1.iter().any(|id| id == own), "{own} not in {u1:?}");
    }
    let h1 = ids("H1");
    assert!(
        h1.iter()
            .any(|id| id == "11111111-0000-4000-8000-000000000003"),
        "{h1:?}"
    );
    assert!(
        h1.iter()
            .any(|id| id == "11111111-0000-4000-8000-0000000000b1"),
        "{h1:?}"
    );
    // U2, a copy of U1's footprint, and the library's C1 and C2 get ids
    // of their own (the KiCad 6 library's tstamps too): no id twice.
    let mut all: Vec<String> = ["U1", "U2", "C1", "C2", "H1"]
        .iter()
        .flat_map(|reference| ids(reference))
        .collect();
    assert!(
        ids("U2").iter().all(|id| !id.starts_with("11111111")),
        "{:?}",
        ids("U2")
    );
    assert!(
        ids("C1").iter().all(|id| !id.starts_with("22222222")),
        "{:?}",
        ids("C1")
    );
    let count = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), count, "an id twice: {all:?}");
    fs::remove_dir_all(&directory).unwrap();
}

fn hostile(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/hostile")
        .join(name)
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pcb-maker-hostile-import-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn schematic_import_copies_the_matching_rules_under_the_board_name() {
    if crate::native_report::check_kicad_cli().is_err() {
        return;
    }
    let directory = scratch("schematic-rules");
    let source = directory.join("source");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/urchin");
    copy_directory_tree(&fixture, &source).unwrap();
    let input = source.join("main.kicad_sch");
    let schematic = fs::read(&input).unwrap();
    let rules =
        "(version 1)\n(rule \"Copper track floor\" (constraint track_width (min 0.22mm)))\n";
    fs::write(source.join("main.kicad_dru"), rules).unwrap();
    fs::write(source.join("other.kicad_dru"), "(version 1)").unwrap();

    let output = directory.join("import");
    import_kicad_netlist(&input, &output, "renamed", 2).unwrap();
    assert_eq!(
        fs::read_to_string(output.join("renamed.kicad_dru")).unwrap(),
        rules
    );
    assert!(!output.join("main.kicad_dru").exists());
    assert!(!output.join("other.kicad_dru").exists());

    // In the source project, a rule already supplied for the output
    // board takes precedence and its original schematic is left intact.
    let caller_rules = "(version 1)\n# caller rules\n";
    fs::write(source.join("renamed.kicad_dru"), caller_rules).unwrap();
    import_kicad_netlist(&input, &source, "renamed", 2).unwrap();
    assert_eq!(
        fs::read_to_string(source.join("renamed.kicad_dru")).unwrap(),
        caller_rules
    );
    assert_eq!(
        fs::read_to_string(source.join("main.kicad_dru")).unwrap(),
        rules
    );
    assert_eq!(fs::read(&input).unwrap(), schematic);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn netlist_import_relaxes_netclass_gaps_and_reports_board_and_custom_minima() {
    if crate::native_report::check_kicad_cli().is_err() {
        return;
    }
    let directory = scratch("pad-rules");
    let library = directory.join("lib.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::write(
        library.join("Jumper.kicad_mod"),
        r#"(footprint "Jumper" (version 20260206) (layer "F.Cu")
            (property "Reference" "REF**" (at 0 -1 0) (layer "F.SilkS")
                (effects (font (size 1 1) (thickness 0.15))))
            (attr smd)
            (pad "1" smd rect (at 0 0) (size 0.6 0.6) (layers "F.Cu" "F.Mask"))
            (pad "2" smd rect (at 0.72 0) (size 0.6 0.6) (layers "F.Cu" "F.Mask")))"#,
    )
    .unwrap();
    let netlist = directory.join("transport.net");
    fs::write(
        &netlist,
        r#"(export (version "E") (design (source "schematic/design.kicad_sch"))
            (components (comp (ref "J1") (value "Jumper") (footprint "lib:Jumper")
                (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
            (nets (net (code "1") (name "A") (node (ref "J1") (pin "1") (pintype "passive")))
                  (net (code "2") (name "B") (node (ref "J1") (pin "2") (pintype "passive")))))"#,
    )
    .unwrap();
    let import = |name: &str, minimum: f64, keep_footprint_clearances: bool| {
        let output = directory.join(name);
        fs::create_dir(&output).unwrap();
        let project = output.join("new.kicad_pro");
        fs::write(
            &project,
            format!(
                r#"{{"board": {{"design_settings": {{"rules": {{"min_clearance": {minimum}}}}}}},
                    "net_settings": {{"meta": {{"version": 3}}, "classes":
                    [{{"name": "Default", "clearance": 0.2, "track_width": 0.2}}]}}}}"#,
            ),
        )
        .unwrap();
        let report = import_kicad_netlist_with_options(
            &netlist,
            &output,
            "new",
            KiCadNetlistImportOptions {
                layers: Some(2),
                keep_footprint_clearances,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.missing.is_empty(), "{:?}", report.missing);
        (report, project)
    };
    let native_pad_findings = |report: &KiCadNetlistImport| {
        let path = report.board.with_extension("drc.json");
        crate::native_report::run_kicad_report(
            &[
                "pcb",
                "drc",
                "--format",
                "json",
                "--severity-all",
                "--output",
            ],
            &path,
            &report.board,
        )
        .unwrap();
        let drc = read_json(&path).unwrap();
        drc["violations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|finding| finding["type"] == "clearance" && finding["severity"] == "error")
            .filter_map(|finding| finding["description"].as_str().map(str::to_string))
            .collect::<Vec<_>>()
    };
    let supplied_project = r#"{
        "board": {"design_settings": {"rules": {"min_clearance": 0.05}}},
        "net_settings": {"meta": {"version": 3}, "classes":
            [{"name": "Default", "clearance": 0.25, "track_width": 0.30}]}
    }"#;
    let companion = netlist.with_extension("kicad_pro");
    fs::write(&companion, supplied_project).unwrap();
    for (output, board_id) in [
        (directory.join("companion-import"), "renamed"),
        (directory.clone(), "transport"),
    ] {
        let report = import_kicad_netlist_with_options(
            &netlist,
            &output,
            board_id,
            KiCadNetlistImportOptions {
                layers: Some(2),
                keep_footprint_clearances: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(output.join(format!("{board_id}.kicad_pro"))).unwrap(),
            supplied_project
        );
        let gap = report.footprint_clearance_evidence[0]
            .netclass_gap
            .as_ref()
            .unwrap();
        assert_eq!(gap.required_mm, 0.25);
        assert!(!gap.applied);
    }
    assert_eq!(fs::read_to_string(&companion).unwrap(), supplied_project);

    let (adjusted, _) = import("netclass", 0.05, false);
    assert_eq!(adjusted.clearances.len(), 1, "{adjusted:?}");
    assert!(adjusted.clearance_conflicts.is_empty(), "{adjusted:?}");
    assert!(native_pad_findings(&adjusted).is_empty());
    assert!(
        adjusted.footprint_clearance_evidence[0]
            .netclass_gap
            .as_ref()
            .unwrap()
            .applied
    );

    let (preserved, _) = import("preserve-netclass", 0.05, true);
    assert_eq!(preserved.footprint_clearance_policy, "preserve_supplied");
    assert!(preserved.clearances.is_empty());
    let witness = &preserved.footprint_clearance_evidence[0];
    assert_eq!(witness.reference, "J1");
    assert!(witness.supplied_footprint_mm.is_none());
    let gap = witness.netclass_gap.as_ref().unwrap();
    assert_eq!(gap.required_mm, 0.2);
    assert!(!gap.applied);
    assert!(preserved.clearance_conflicts[0].contains("Preserved the supplied clearance policy"));
    assert_eq!(native_pad_findings(&preserved).len(), 1);
    let board = fs::read_to_string(&preserved.board).unwrap();
    assert!(!board.contains("(clearance "));

    let jumper_path = library.join("Jumper.kicad_mod");
    let jumper = fs::read_to_string(&jumper_path).unwrap();
    fs::write(
        &jumper_path,
        jumper.replace("(attr smd)", "(attr smd) (clearance 0.10)"),
    )
    .unwrap();
    let (authored, _) = import("authored-override", 0.05, true);
    assert!(authored.clearances.is_empty());
    assert_eq!(
        authored.footprint_clearance_evidence[0].supplied_footprint_mm,
        Some(0.1)
    );
    assert!(
        authored.footprint_clearance_evidence[0]
            .netclass_gap
            .is_none()
    );
    assert!(native_pad_findings(&authored).is_empty());
    assert!(
        fs::read_to_string(&authored.board)
            .unwrap()
            .contains("(clearance 0.10)")
    );

    fs::write(&jumper_path, jumper.replace("(at 0.72 0)", "(at 1 0)")).unwrap();
    let (sufficient, _) = import("adequate-gap", 0.05, true);
    let (sufficient_default, _) = import("adequate-gap-default", 0.05, false);
    assert!(sufficient.clearances.is_empty() && sufficient.footprint_clearance_evidence.is_empty());
    assert_eq!(
        fs::read(&sufficient.board).unwrap(),
        fs::read(&sufficient_default.board).unwrap()
    );
    fs::write(&jumper_path, jumper).unwrap();

    let (board_minimum, _) = import("board-minimum", 0.15, true);
    assert!(board_minimum.clearances.is_empty(), "{board_minimum:?}");
    assert_eq!(
        board_minimum.clearance_conflicts.len(),
        1,
        "{board_minimum:?}"
    );
    assert!(
        board_minimum.clearance_conflicts[0]
            .contains("board minimum (Board Setup) requires 0.15 mm")
    );
    assert_eq!(native_pad_findings(&board_minimum).len(), 1);

    // A netlist exported under another name points to the actual board's
    // rules. KiCad uses the clearance rule in the temporary check and on
    // the imported board. The router resolves the same file's width.
    fs::create_dir(directory.join("schematic")).unwrap();
    let custom_rules = r#"(version 1)
        (rule "Higher pad clearance" (condition "A.Type == 'Pad' && B.Type == 'Pad'")
            (constraint clearance (min 0.3mm)))
        (rule "Wide A" (condition "A.NetName == 'A'") (constraint track_width (min 0.55mm)))"#;
    fs::write(directory.join("schematic/design.kicad_dru"), custom_rules).unwrap();
    let (custom, project) = import("custom", 0.05, true);
    assert!(custom.clearances.is_empty(), "{custom:?}");
    assert_eq!(custom.clearance_conflicts.len(), 1, "{custom:?}");
    assert!(custom.clearance_conflicts[0].contains("rule 'Higher pad clearance' requires 0.3 mm"));
    assert_eq!(
        fs::read_to_string(project.with_extension("kicad_dru")).unwrap(),
        custom_rules
    );
    assert!(
        native_pad_findings(&custom)
            .iter()
            .any(|description| description.contains("rule 'Higher pad clearance'"))
    );
    let router = resolve_project_rules(&project, &custom.board).unwrap();
    assert_eq!(router.connection_rules["A"].trace_width_mm, 0.55);
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn footprint_ids_and_names_never_make_paths_outside_their_folders() {
    let directory = scratch("paths");
    // 1.2: a netlist's `lib:../evil` reads no file beside the library.
    let library = directory.join("lib.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::write(library.join("R.kicad_mod"), "(footprint \"R\")").unwrap();
    fs::write(directory.join("evil.kicad_mod"), "(footprint \"evil\")").unwrap();
    let libraries = BTreeMap::from([("lib".to_string(), library.clone())]);
    assert!(find_footprint(&libraries, "lib:R").is_some());
    assert!(find_footprint(&libraries, "lib:../evil").is_none());
    assert!(find_footprint(&libraries, "lib:..").is_none());
    // 1.1: a --footprints-from board whose footprint id climbs out of
    // the output directory: refused, nothing written outside it.
    let netlist = directory.join("t.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "J1") (value "X") (footprint "../../x:../../y") (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
          (nets (net (code "1") (name "/TARGET") (node (ref "J1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let output = directory.join("a/b/out");
    let error = import_kicad_netlist_from(
        &netlist,
        &output,
        "t",
        2,
        Some(&hostile("climbing-footprint-id.kicad_pcb")),
    )
    .unwrap_err();
    assert!(error.contains("its library nickname or name"), "{error}");
    assert!(crate::exit::is_input_error(&error));
    assert!(!directory.join("a/x.pretty").exists());
    assert!(!directory.join("x.pretty").exists());
    // A board id that climbs: refused before anything is written.
    let error = import_kicad_netlist(&netlist, &directory.join("c"), "../t", 2).unwrap_err();
    assert!(error.contains("the board id"), "{error}");
    assert!(!directory.join("c").exists());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_library_footprint_without_its_name_is_reported_missing_not_a_panic() {
    // 2.1: `(footprint)` in a project's .pretty failed the whole import
    // (index out of bounds).
    let directory = scratch("short");
    let library = directory.join("lib.pretty");
    fs::create_dir_all(&library).unwrap();
    fs::copy(
        hostile("short-footprint.kicad_mod"),
        library.join("R.kicad_mod"),
    )
    .unwrap();
    let netlist = directory.join("t.net");
    fs::write(
        &netlist,
        r#"(export (version "E")
          (components
            (comp (ref "R1") (value "1k") (footprint "lib:R") (sheetpath (names "/") (tstamps "/")) (tstamps "1111")))
          (nets (net (code "1") (name "A") (node (ref "R1") (pin "1") (pintype "passive")))))"#,
    )
    .unwrap();
    let report = import_kicad_netlist(&netlist, &directory.join("out"), "t", 2).unwrap();
    assert_eq!(report.missing.len(), 1, "{:?}", report.missing);
    assert!(
        report.missing[0].contains("the file holds a footprint form without its name"),
        "{:?}",
        report.missing
    );
    // From a --footprints-from board: the input's error.
    let error = import_kicad_netlist_from(
        &netlist,
        &directory.join("out2"),
        "t",
        2,
        Some(&hostile("short-footprint.kicad_pcb")),
    )
    .unwrap_err();
    assert!(
        error.contains("a footprint form without its name"),
        "{error}"
    );
    assert!(crate::exit::is_input_error(&error));
    // And the forms that index past the name refuse it.
    let short = parse("(footprint)").unwrap();
    assert!(crate::board_footprints::library_form(&short, "L:R").is_err());
    assert!(crate::board_footprints::posed(&short, [0.0; 3]).is_err());
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn a_library_that_holds_the_output_or_links_out_is_not_copied() {
    // 1.3: `${KIPRJMOD}/.` (or a link `lib -> .`) copied the project
    // into its own output, again and again.
    let directory = scratch("copy");
    let project = directory.join("project");
    let output = project.join("out");
    fs::create_dir_all(project.join("lib.pretty")).unwrap();
    fs::create_dir_all(&output).unwrap();
    fs::write(project.join("lib.pretty/R.kicad_mod"), "(footprint \"R\")").unwrap();
    fs::write(directory.join("secret.kicad_mod"), "(footprint \"S\")").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&project, project.join("lib.pretty/loop")).unwrap();
        std::os::unix::fs::symlink(
            directory.join("secret.kicad_mod"),
            project.join("lib.pretty/secret.kicad_mod"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&directory, project.join("outside.pretty")).unwrap();
    }
    // The project itself: not copied.
    copy_library_tree(&project.join("."), &output.join("."), &project, &output).unwrap();
    assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    // An ordinary library: its files, not its links.
    copy_library_tree(
        &project.join("lib.pretty"),
        &output.join("lib.pretty"),
        &project,
        &output,
    )
    .unwrap();
    assert!(output.join("lib.pretty/R.kicad_mod").is_file());
    assert!(!output.join("lib.pretty/loop").exists());
    assert!(!output.join("lib.pretty/secret.kicad_mod").exists());
    // A link out of the project: not copied.
    #[cfg(unix)]
    {
        copy_library_tree(
            &project.join("outside.pretty"),
            &output.join("outside.pretty"),
            &project,
            &output,
        )
        .unwrap();
        assert!(!output.join("outside.pretty").exists());
    }
    fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn the_copper_layers_come_from_layers_else_the_default() {
    assert_eq!(copper_layers(Some(4), None), (4, "--layers".to_string()));
    assert_eq!(
        copper_layers(None, None),
        (2, "the default (--layers sets another)".to_string())
    );
}

#[test]
fn pins_are_read_with_their_nets_and_unnamed_nets_are_skipped() {
    let netlist = parse(
        r#"(export (nets
            (net (code "1") (name "GND")
                (node (ref "R1") (pin "1") (pinfunction "A") (pintype "passive"))
                (node (ref "R2") (pin "2")))
            (net (code "2"))))"#,
    )
    .unwrap();
    let (pins, nets) = pin_nets(&netlist);
    assert_eq!(nets, 1);
    assert_eq!(pins.len(), 2);
    assert_eq!(
        pins[&("R1".to_string(), "1".to_string())],
        vec![(
            "GND".to_string(),
            Some("A".to_string()),
            Some("passive".to_string())
        )]
    );
    assert_eq!(
        pins[&("R2".to_string(), "2".to_string())],
        vec![("GND".to_string(), None, None)]
    );
}

#[test]
fn a_symbols_path_is_its_sheets_path_and_its_own_stamp() {
    let nested =
        parse(r#"(comp (ref "R1") (sheetpath (names "/a/") (tstamps "/s1/")) (tstamps "u1"))"#)
            .unwrap();
    assert_eq!(symbol_path_of(&nested).as_deref(), Some("/s1/u1"));
    let root = parse(r#"(comp (ref "R2") (tstamp "u2"))"#).unwrap();
    assert_eq!(symbol_path_of(&root).as_deref(), Some("/u2"));
    let stampless = parse(r#"(comp (ref "R3"))"#).unwrap();
    assert_eq!(symbol_path_of(&stampless), None);
}

#[test]
fn drifted_footprints_list_six_references_and_then_dots() {
    let references: Vec<String> = (1..=7).map(|n| format!("SW{n}")).collect();
    let drift = BTreeMap::from([(
        ("lib:SW".to_string(), "board:SW".to_string()),
        ("library 1 x 1 mm".to_string(), references),
    )]);
    assert_eq!(
        drifted_lines(drift),
        vec!["lib:SW (7 parts: SW1, SW2, SW3, SW4, SW5, SW6, ...): library 1 x 1 mm".to_string()]
    );
}

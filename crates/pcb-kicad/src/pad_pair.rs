// Copyright (C) 2026 Toit contributors.

//! A local routing question with the full native board retained as context.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPadReference {
    pub footprint: String,
    pub pad: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KiCadPadPairRoutingConfig {
    pub connection: String,
    pub start: KiCadPadReference,
    pub finish: KiCadPadReference,
    pub routing: KiCadGridRouteConfig,
}

#[derive(Debug, Serialize)]
pub struct KiCadPadPairRouteProposal {
    pub schema_version: u32,
    pub source_board_sha256: String,
    pub config: KiCadPadPairRoutingConfig,
    pub route_found: bool,
    /// Only new bridge copper. Never apply as a whole-net replacement.
    pub bridge: Option<KiCadRouteCandidate>,
    pub error: Option<String>,
    pub search_failure: Option<KiCadRouteSearchFailure>,
    pub native_admitted: bool,
    pub source_unchanged: bool,
    pub render_error: Option<String>,
    pub contract: &'static str,
}

pub(super) fn select_terminals(
    pcb: &Expr,
    model: &mut KiCadRoutingModel,
    pair: &KiCadPadPairRoutingConfig,
) -> Result<Vec<ElectricalTerminal>, String> {
    if pair.connection.is_empty() || normalize_net(&pair.connection) != pair.connection {
        return Err("pad-pair connection must be nonempty and canonical".into());
    }
    let mut selected = Vec::new();
    for reference in [&pair.start, &pair.finish] {
        let footprints: Vec<_> = pcb
            .children()
            .iter()
            .filter(|fp| {
                fp.head() == Some("footprint")
                    && footprint_reference(fp).as_deref() == Some(reference.footprint.as_str())
            })
            .collect();
        if footprints.len() != 1 {
            return Err(format!(
                "pad-pair footprint {:?} must resolve uniquely",
                reference.footprint
            ));
        }
        let fp = footprints[0];
        let pads: Vec<_> = fp
            .children()
            .iter()
            .filter(|p| {
                p.head() == Some("pad")
                    && p.children().get(1).and_then(Expr::atom) == Some(reference.pad.as_str())
            })
            .collect();
        if pads.len() != 1 {
            return Err(format!(
                "pad-pair pad {:?}-{:?} must resolve uniquely",
                reference.footprint, reference.pad
            ));
        }
        if node_net(pads[0]).map(normalize_net) != Some(pair.connection.as_str()) {
            return Err("pad-pair endpoint belongs to a different net".into());
        }
        let fp_at = form_at(fp)?;
        let pad_at = form_at(pads[0])?;
        let offset = rotate_vector([pad_at[0], pad_at[1]], -fp_at[2]);
        let at = [fp_at[0] + offset[0], fp_at[1] + offset[1]];
        let layers = pad_copper_layers(pads[0]);
        let contacts: Vec<_> = model
            .electrical_terminals()
            .into_iter()
            .filter(|contact| contact.at == at && (0..2).all(|i| !layers[i] || contact.layers[i]))
            .collect();
        if layers == [false; 2] || contacts.len() != 1 {
            return Err(
                "pad-pair anchor has missing or ambiguous electrical layer contacts".into(),
            );
        }
        // A same-net plated pad can join a coincident SMD pad to both layers.
        // Separate unplated F/B pads instead resolve to their exact layer goal.
        selected.push(contacts[0]);
    }
    if selected[0] == selected[1] {
        return Err("pad-pair endpoints must have distinct electrical contacts".into());
    }
    // Other same-net pads and drill obstacles retain their geometry; only the
    // mandatory routing goals change. Never relabel or delete source pads.
    model.terminals = selected.iter().map(|t| t.at).collect();
    model.terminals.dedup();
    Ok(selected)
}

fn append_bridge(pcb: &mut Expr, candidate: &KiCadRouteCandidate) -> Result<(), String> {
    let names = unambiguous_pad_net_names(pcb)?;
    let mut additions = Expr::List(
        candidate
            .supplemental_segments
            .iter()
            .map(supplemental_segment_expr)
            .chain(
                candidate
                    .supplemental_vias
                    .iter()
                    .map(supplemental_via_expr),
            )
            .collect(),
    );
    // Canonicalize only new copper. Existing net spelling is source design too.
    canonicalize_copper_net_names(&mut additions, &names);
    let (Expr::List(items), Expr::List(additions)) = (pcb, additions) else {
        return Err("PCB root is not a list".into());
    };
    items.extend(additions);
    Ok(())
}

pub fn propose_kicad_pad_pair_route(
    board: &Path,
    config: &KiCadPadPairRoutingConfig,
    output: &Path,
) -> Result<KiCadPadPairRouteProposal, String> {
    validate_grid_route_config(&config.routing)?;
    let source = fs::read_to_string(board).map_err(|e| e.to_string())?;
    let source_hash = format!("{:x}", Sha256::digest(source.as_bytes()));
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(output).map_err(|e| format!("pad-pair output must be new: {e}"))?;
    let preview = output.join("preview.kicad_pcb");
    fs::write(&preview, &source).map_err(|e| e.to_string())?;
    let mut result = KiCadPadPairRouteProposal {
        schema_version: 1,
        source_board_sha256: source_hash.clone(),
        config: config.clone(),
        route_found: false,
        bridge: None,
        error: None,
        search_failure: None,
        native_admitted: false,
        source_unchanged: true,
        render_error: None,
        contract: "Diagnostic path between two named existing pad anchors; all other pads, holes and foreign copper retained as context. Preview appends bridge copper without removing any source item. No island-separation proof, full-net completion or native admission. Bridge is not a whole-net replacement candidate.",
    };
    match route_materialized_connection_selected(
        &preview,
        &config.connection,
        &[],
        &config.routing,
        None,
        Some(config),
    ) {
        Ok(candidate) => {
            let mut pcb = parse(&source)?;
            append_bridge(&mut pcb, &candidate)?;
            fs::write(&preview, format!("{}\n", encode(&pcb))).map_err(|e| e.to_string())?;
            result.route_found = true;
            result.bridge = Some(candidate);
        }
        Err(error) => {
            result.search_failure = error.search_failure().cloned();
            result.error = Some(error.to_string());
        }
    }
    result.source_unchanged = file_sha256(board)? == source_hash;
    result.render_error = verification_preview::render(&preview, output).err();
    write_typed_json(&output.join("proposal.json"), &result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(back: bool) -> Expr {
        parse(&format!(
            r#"(kicad_pcb
            (gr_rect (start 0 0) (end 12 8) (layer "Edge.Cuts"))
            (footprint "A" (at 2 2) (property "Reference" "A")
              (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "N")))
            (footprint "B" (at 9 2) (property "Reference" "B")
              (pad "1" smd circle (at 0 0) (size 1 1) (layers "{}") (net "N")))
            (footprint "C" (at 9 6) (property "Reference" "C")
              (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill 0.4) (layers "*.Cu") (net "N")))
            (footprint "X" (at 5 2) (property "Reference" "X")
              (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "X"))))"#,
            if back { "B.Cu" } else { "F.Cu" }
        ))
        .unwrap()
    }
    fn config() -> KiCadPadPairRoutingConfig {
        KiCadPadPairRoutingConfig {
            connection: "N".into(),
            start: KiCadPadReference {
                footprint: "A".into(),
                pad: "1".into(),
            },
            finish: KiCadPadReference {
                footprint: "B".into(),
                pad: "1".into(),
            },
            routing: KiCadGridRouteConfig {
                resolution_mm: 0.25,
                trace_width_mm: 0.2,
                clearance_mm: 0.1,
                ..Default::default()
            },
        }
    }
    #[test]
    fn named_pair_keeps_other_pad_hole_and_foreign_geometry() {
        let pcb = fixture(true);
        let mut model = KiCadRoutingModel::from_pcb(&pcb, "N").unwrap();
        let before = format!(
            "{:?}{:?}{:?}",
            model.terminal_pads, model.target_holes, model.obstacles
        );
        select_terminals(&pcb, &mut model, &config()).unwrap();
        assert_eq!(model.terminals, vec![[2.0, 2.0], [9.0, 2.0]]);
        assert_eq!(
            format!(
                "{:?}{:?}{:?}",
                model.terminal_pads, model.target_holes, model.obstacles
            ),
            before
        );
        assert_eq!(
            model
                .electrical_terminals()
                .iter()
                .map(|t| t.layers)
                .collect::<Vec<_>>(),
            vec![[true, false], [false, true]]
        );
    }
    #[test]
    fn invalid_or_foreign_pad_identity_cannot_select_a_goal() {
        let pcb = fixture(false);
        for (reference, pad, net) in [
            ("missing", "1", "N"),
            ("X", "1", "N"),
            ("A", "wrong", "N"),
            ("A", "1", "/N"),
            ("A", "1", ""),
        ] {
            let mut pair = config();
            pair.start.footprint = reference.into();
            pair.start.pad = pad.into();
            pair.connection = net.into();
            assert!(
                select_terminals(
                    &pcb,
                    &mut KiCadRoutingModel::from_pcb(&pcb, "N").unwrap(),
                    &pair
                )
                .is_err()
            );
        }
        let mut pair = config();
        pair.finish = pair.start.clone();
        assert!(
            select_terminals(
                &pcb,
                &mut KiCadRoutingModel::from_pcb(&pcb, "N").unwrap(),
                &pair
            )
            .is_err()
        );
    }
    #[test]
    fn named_smd_at_plated_anchor_resolves_to_connected_group() {
        let pcb = parse(
            r#"(kicad_pcb
          (gr_rect (start 0 0) (end 12 8) (layer "Edge.Cuts"))
          (footprint A (at 2 2) (property "Reference" "A")
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "N")))
          (footprint B (at 2 2) (property "Reference" "B")
            (pad "1" thru_hole circle (at 0 0) (size 1 1) (drill .4) (layers "*.Cu") (net "N")))
          (footprint C (at 9 2) (property "Reference" "C")
            (pad "1" smd circle (at 0 0) (size 1 1) (layers "F.Cu") (net "N"))))"#,
        )
        .unwrap();
        let mut request = config();
        request.finish.footprint = "C".into();
        let mut selected = Vec::new();
        for reference in ["A", "B"] {
            request.start.footprint = reference.into();
            let mut model = KiCadRoutingModel::from_pcb(&pcb, "N").unwrap();
            selected.push(select_terminals(&pcb, &mut model, &request).unwrap());
        }
        assert_eq!(selected[0], selected[1]);
        assert_eq!(selected[0][0].layers, [true, true]);
        request.start.footprint = "A".into();
        request.finish.footprint = "B".into();
        assert!(
            select_terminals(
                &pcb,
                &mut KiCadRoutingModel::from_pcb(&pcb, "N").unwrap(),
                &request
            )
            .is_err()
        );
    }
    #[test]
    fn pair_routes_only_selected_goals_and_keeps_actual_layers() {
        let directory = env::temp_dir().join(format!(
            "pcb-maker-pad-pair-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let board = directory.join("board.kicad_pcb");
        let pcb = fixture(true);
        fs::write(&board, encode(&pcb)).unwrap();
        let pair = config();
        let full = route_materialized_connection(&board, "N", &pair.routing).unwrap();
        let selected = route_materialized_connection_selected(
            &board,
            "N",
            &[],
            &pair.routing,
            None,
            Some(&pair),
        )
        .unwrap();
        assert_eq!(full.terminals.len(), 3);
        assert_eq!(selected.terminals.len(), 2);
        assert_eq!(selected.branches.len(), 1);
        assert!(!selected.supplemental_vias.is_empty());
        assert_eq!(parse(&fs::read_to_string(&board).unwrap()).unwrap(), pcb);
        let mut retained = pcb.clone();
        if let Expr::List(items) = &mut retained {
            items.push(parse("(segment (start 1 1) (end 1 2) (net N) (uuid original))").unwrap());
        }
        let original = retained.children().to_vec();
        append_bridge(&mut retained, &selected).unwrap();
        assert_eq!(&retained.children()[..original.len()], original.as_slice());
        assert!(retained.children().len() > original.len());
        // Coincident opposite-layer pads remain two electrical goals; selecting
        // their common XY must not imply a plated connection or include C.
        fs::write(&board, encode(&pcb).replace("(at 9 2)", "(at 2 2)")).unwrap();
        let coincident = route_materialized_connection_selected(
            &board,
            "N",
            &[],
            &pair.routing,
            None,
            Some(&pair),
        )
        .unwrap();
        assert_eq!(coincident.terminals, vec![[2.0, 2.0], [2.0, 2.0]]);
        assert_eq!(coincident.branches.len(), 1);
        assert!(!coincident.supplemental_vias.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }
}

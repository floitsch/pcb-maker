// Copyright (C) 2026 Toit contributors.

//! A board placed in pieces: a panel's copies (each seed places the first
//! and replicates it) and the sub-boards of a panel of different boards
//! (their parts held in their regions, their frames and tabs kept clear).

use super::*;

/// How the board is placed in pieces (`place_in_pieces`).
pub(super) struct BoardPieces {
    /// A panel's copies (`panel_copies`): each seed places the first copy
    /// and replicates it (`panel::place_copies`).
    pub copies: Option<panel::PanelCopies>,
    /// The sub-boards when the placement holds their parts in them: each
    /// seed places them one by one (`sub_boards::place_sub_boards`).
    pub held_sub_boards: Option<crate::sub_boards::PlacerSubBoards>,
    /// board-placer.json's `sub_boards`.
    pub report: Option<crate::sub_boards::KiCadSubBoardsReport>,
}

/// Finds the panel's copies and the sub-boards, and changes the problem
/// as they place it: the copies' parts kept off the frames, a sub-board's
/// parts held in its region, the nets the router splits noted.
pub(super) fn place_in_pieces(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &mut LoweredPlacement,
) -> Result<BoardPieces, String> {
    let config = &context.config;
    let copies = if config.panel_copies {
        detect_copies(pcb, lowered)
    } else {
        None
    };
    // Sub-boards of a panel of different boards (`sub_boards`): where no
    // copies were found (`sub_boards::detect_for_placer`), before the
    // escape demand and the crossings read the problem's nets.
    let mut pieces = BoardPieces {
        copies,
        held_sub_boards: None,
        report: None,
    };
    crate::outline_pieces::note_designed(None);
    // A panel of copies whose cells share net names (the PolyKybd
    // molecules: every cell's 27 nets, no copper between cells): the
    // router splits those nets per cell and KiCad's items across cells
    // are not counted (`outline_pieces::note_cells`).
    if let Some(copies) = &pieces.copies
        && copies.detected_by == "repeated references"
    {
        let shared = crate::outline_pieces::repeated_reference_nets(pcb);
        if !shared.is_empty() {
            eprintln!(
                "placer: panel copies: {} nets of one name in several cells: one net per cell",
                shared.len()
            );
            crate::outline_pieces::note_designed(Some(shared));
            crate::outline_pieces::note_cells(Some(copies.regions.clone()));
        }
    }
    if config.sub_boards {
        place_sub_boards(config, pcb, lowered, &mut pieces)?;
    }
    Ok(pieces)
}

/// A panel's copies (`panel::detect`), said on stderr; none where the
/// board is not one.
fn detect_copies(pcb: &Expr, lowered: &LoweredPlacement) -> Option<panel::PanelCopies> {
    let footprint_count = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .count();
    match panel::detect(&lowered.problem, footprint_count) {
        Ok(copies) => {
            eprintln!(
                "placer: panel copies: {} copies of {} parts ({}, regions from {})",
                copies.members.len(),
                copies.members[0].len(),
                copies.detected_by,
                copies.regions_from
            );
            if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                eprintln!(
                    "placer: panel copies: regions {:?}",
                    copies
                        .regions
                        .iter()
                        .map(|r| r.map(|v| (v * 100.0).round() / 100.0))
                        .collect::<Vec<_>>()
                );
            }
            Some(copies)
        }
        Err(why) => {
            if std::env::var_os("PCB_PLACER_DEBUG").is_some() {
                eprintln!("placer: panel copies: none ({why})");
            }
            None
        }
    }
}

/// The sub-boards (`sub_boards`): detected on the board (with the
/// reference board's help where one is given), then a panel's copies kept
/// off their frames, or the sub-boards' parts held in their regions.
fn place_sub_boards(
    config: &KiCadBoardPlacerConfig,
    pcb: &Expr,
    lowered: &mut LoweredPlacement,
    pieces: &mut BoardPieces,
) -> Result<(), String> {
    let footprint_count = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .count();
    let reference = match &config.sub_board_reference {
        Some(path) => {
            let text = fs::read_to_string(path).map_err(|error| {
                crate::exit::input_error(format!(
                    "failed to read the reference board {}: {error}",
                    path.display()
                ))
            })?;
            Some(crate::canonical_order::parse_board(&text).map_err(crate::exit::input_error)?)
        }
        None => None,
    };
    let detected = crate::sub_boards::detect_for_placer(
        pcb,
        &lowered.problem,
        footprint_count,
        reference.as_ref(),
    )?;
    // No part on any piece (a netlist import's stack without
    // `--reference`): the pieces, tabs and frames from the outline
    // alone are reported, nothing else changes.
    if pieces.copies.is_none()
        && let Some(sub) = &detected
        && sub.count(&lowered.problem) == 0
        && !sub.has_frames()
    {
        let report = crate::sub_boards::KiCadSubBoardsReport::of_outline(
            sub,
            &lowered.problem,
            &lowered.references,
        );
        eprintln!(
            "placer: sub-boards: the outline falls into {} pieces (cut by {}; {} frames or rails by their shape) and no part lies on any: {} movable parts without a board (give --reference or a region per board)",
            report.pieces,
            report.cut_by.join(", "),
            report.frames.len(),
            report.unassigned
        );
        pieces.report = Some(report);
    }
    let sub = detected.filter(|sub| sub.count(&lowered.problem) >= 2 || sub.has_frames());
    if let (Some(copies), Some(sub)) = (&pieces.copies, &sub) {
        pieces.report = Some(keep_copies_off_frames(copies, sub, lowered));
    }
    if pieces.copies.is_none()
        && let Some(sub) = sub
    {
        let (report, held) = hold_in_sub_boards(pcb, sub, lowered);
        pieces.report = Some(report);
        pieces.held_sub_boards = held;
    }
    Ok(())
}

/// A panel of copies keeps its parts off its frames and tabs too, each
/// copy off the images of every copy's (one placement replicates to all),
/// but for connectors and the designer's overhangs
/// (`sub_boards::keep_off_frames`); the copies are placed as copies,
/// nothing held by sub-board, no net split.
fn keep_copies_off_frames(
    copies: &panel::PanelCopies,
    sub: &crate::sub_boards::PlacerSubBoards,
    lowered: &mut LoweredPlacement,
) -> crate::sub_boards::KiCadSubBoardsReport {
    let frames = crate::outline_pieces::CopyFrames {
        transforms: copies
            .transforms
            .iter()
            .map(|transform| (transform.angle, transform.offset))
            .collect(),
        within: copies_surroundings(copies, sub),
        held: copies_held_bands(copies, &lowered.problem),
    };
    let names = lowered.references.clone();
    let mut applied = crate::sub_boards::PlacerApplied::default();
    crate::sub_boards::keep_off_frames(
        sub,
        &mut lowered.problem,
        &mut lowered.references,
        &mut lowered.source_at,
        &mut applied,
        true,
        Some(&frames),
    );
    let mut report = crate::sub_boards::KiCadSubBoardsReport::of(
        sub,
        &lowered.problem,
        &lowered.references,
        &names,
        &applied,
    );
    report.copies = true;
    eprintln!(
        "placer: sub-boards: a panel of {} copies, {} frames, cut by {}, {} keepouts over the tabs and frames ({:.0} mm2, each copy's in every copy; margin {}, grown by 1.5 mm they take {:.2} of the first copy's surroundings), {} connectors and {} designer's overhangs may reach over them",
        copies.members.len(),
        report.frames.len(),
        report.cut_by.join(", "),
        applied.tabs,
        applied.kept_clear_mm2,
        match applied.margin_mm {
            Some(margin) => format!("{margin} mm"),
            None => "none: the copies are too small for it, nothing kept off".into(),
        },
        applied.copy_kept_share,
        applied.connectors,
        applied.designer_overhangs
    );
    report
}

/// The sub-boards of a panel of different boards, their parts held in
/// their regions (`sub_boards::apply_to_placer`): the report, and the
/// sub-boards where the placement holds any part in one.
fn hold_in_sub_boards(
    pcb: &Expr,
    sub: crate::sub_boards::PlacerSubBoards,
    lowered: &mut LoweredPlacement,
) -> (
    crate::sub_boards::KiCadSubBoardsReport,
    Option<crate::sub_boards::PlacerSubBoards>,
) {
    // A board beside its frame alone (one sub-board) has no nets on
    // several by design.
    if sub.count(&lowered.problem) >= 2 {
        crate::outline_pieces::note_designed(Some(crate::sub_boards::designed_nets(
            pcb,
            &sub,
            &lowered.net_names,
        )));
    }
    let names = lowered.references.clone();
    let applied = crate::sub_boards::apply_to_placer(
        &sub,
        &mut lowered.problem,
        &mut lowered.references,
        &mut lowered.source_at,
        &mut lowered.poured,
    );
    let report = crate::sub_boards::KiCadSubBoardsReport::of(
        &sub,
        &lowered.problem,
        &lowered.references,
        &names,
        &applied,
    );
    eprintln!(
        "placer: sub-boards: {} ({}), {} nets on several, {} parts without one, {} held in their region, {} frames, cut by {}, {} keepouts over the tabs and frames ({:.0} mm2), {} connectors and {} designer's overhangs may reach over them",
        report.count,
        report.from.join(", "),
        report.split_nets,
        report.unassigned,
        applied.held,
        report.frames.len(),
        report.cut_by.join(", "),
        applied.tabs,
        applied.kept_clear_mm2,
        applied.connectors,
        applied.designer_overhangs
    );
    let held = (applied.held > 0).then_some(sub);
    (report, held)
}

/// Per copy of a panel, where its own parts may lie: its region, cut to
/// the box of the outline pieces its parts lie on (`sub_boards`; the
/// region alone where none does) grown by the tabs' margin: the area
/// whose keepouts every copy takes over (`Pieces::copies_keepouts`).
pub(super) fn copies_surroundings(
    copies: &panel::PanelCopies,
    sub: &crate::sub_boards::PlacerSubBoards,
) -> Vec<[f64; 4]> {
    let margin = crate::outline_pieces::TAB_MARGIN_MM;
    copies
        .members
        .iter()
        .zip(&copies.regions)
        .map(|(members, region)| {
            let pieces: BTreeSet<usize> = members
                .iter()
                .filter_map(|part| sub.part_piece.get(*part).copied().flatten())
                .collect();
            let Some(around) =
                pieces
                    .iter()
                    .map(|piece| sub.pieces.boxes[*piece])
                    .reduce(|a, b| {
                        [
                            a[0].min(b[0]),
                            a[1].min(b[1]),
                            a[2].max(b[2]),
                            a[3].max(b[3]),
                        ]
                    })
            else {
                return *region;
            };
            [
                region[0].max(around[0] - margin),
                region[1].max(around[1] - margin),
                region[2].min(around[2] + margin),
                region[3].min(around[3] + margin),
            ]
        })
        .collect()
}

/// Per copy of a panel, the bands along the board's edges where one of
/// its movable parts is held (an edge or overhang constraint): as deep as
/// the part's longer side and its edge distance (`max_mm`; the overhang's
/// box), a millimetre more. The other copies' keepouts are not carried
/// there (`outline_pieces::CopyFrames::held`).
pub(super) fn copies_held_bands(
    copies: &panel::PanelCopies,
    problem: &core::Problem,
) -> Vec<Vec<[f64; 4]>> {
    let board = problem.bounds();
    let band = |edge: core::constraints::Edge, depth: f64| match edge {
        core::constraints::Edge::Left => [board[0], board[1], board[0] + depth, board[3]],
        core::constraints::Edge::Right => [board[2] - depth, board[1], board[2], board[3]],
        core::constraints::Edge::Top => [board[0], board[1], board[2], board[1] + depth],
        core::constraints::Edge::Bottom => [board[0], board[3] - depth, board[2], board[3]],
    };
    copies
        .members
        .iter()
        .map(|members| {
            let mut bands = Vec::new();
            for &part in members {
                let component = &problem.components[part];
                if component.fixed {
                    continue;
                }
                let longer = component.body_size[0].max(component.body_size[1]);
                for (held, edge, max) in &problem.constraints.edges {
                    if *held == part {
                        bands.push(band(*edge, longer + max + 1.0));
                    }
                }
                for (held, edge, _) in &problem.constraints.overhangs {
                    if *held == part {
                        bands.push(band(*edge, longer + 1.0));
                    }
                }
            }
            bands
        })
        .collect()
}

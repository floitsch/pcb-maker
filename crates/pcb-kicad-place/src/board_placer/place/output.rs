// Copyright (C) 2026 Toit contributors.

//! The placement written: the poses into the board, every pad checked
//! where the placer believed it is, and the placed directory (the board,
//! the placement's playback).

use super::*;

/// What writing the poses tells the report: per footprint, where it was
/// and where it is now, and its uuid.
pub(super) struct WrittenPoses {
    pub footprints: Vec<KiCadPlacedFootprint>,
    pub uuids: Vec<Option<String>>,
}

/// Writes `placement`'s poses into the board's footprints, and drops the
/// board's tracks and vias.
pub(super) fn write_poses(
    pcb: &mut Expr,
    lowered: &LoweredPlacement,
    placement: &core::Placement,
) -> Result<WrittenPoses, String> {
    let Expr::List(items) = pcb else {
        return Err("PCB root is not a list".into());
    };
    items.retain(|item| !matches!(item.head(), Some("segment" | "arc" | "via")));
    let mut footprints = Vec::new();
    let mut uuids = Vec::new();
    for (index, footprint) in items
        .iter_mut()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        uuids.push(crate::kicad_nets::footprint_uuid(footprint));
        let pose = placement.poses[index];
        let source_at = lowered.source_at[index];
        let at = write_footprint_pose(footprint, pose)?;
        footprints.push(KiCadPlacedFootprint {
            reference: lowered.references[index].clone(),
            fixed: lowered.problem.components[index].fixed,
            halo_mm: lowered.problem.components[index].halo,
            body_center: lowered.problem.components[index].center(pose),
            body_half: lowered.problem.components[index].half_extent(pose.angle),
            source_at,
            at,
        });
    }
    Ok(WrittenPoses { footprints, uuids })
}

/// The written board must put every pad where the placer believed it is.
pub(super) fn check_written_pads(
    pcb: &Expr,
    lowered: &LoweredPlacement,
    poses: &[core::Pose],
) -> Result<(), String> {
    for (index, footprint) in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .enumerate()
    {
        let footprint_at = form_at(footprint)?;
        let component = &lowered.problem.components[index];
        let mut pin = 0;
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if !node_net(pad).is_some_and(placer_net) {
                continue;
            }
            let written = lower_pad(pad, footprint_at)?.center;
            // A part a row carries has no pins of its own: its pads follow
            // its own pose.
            let expected = match component.pins.get(pin) {
                Some(own) => component.pin_position(own, poses[index]),
                None => {
                    let local = form_at(pad)?;
                    component.pin_position(
                        &core::Pin {
                            offset: [local[0], local[1]],
                            net: 0,
                        },
                        poses[index],
                    )
                }
            };
            if distance_squared(written, expected).sqrt() > 1.0e-5 {
                return Err(format!(
                    "internal error: pad of {} written at {written:?}, expected {expected:?}",
                    lowered.references[index]
                ));
            }
            pin += 1;
        }
    }
    Ok(())
}

/// Copies the source directory to the output directory (which must not
/// exist yet) and writes the placed board and the placement's playback
/// (`placement.html`) into it.
pub(super) fn write_placed_directory(
    context: &PlaceContext,
    pcb: &Expr,
    lowered: &LoweredPlacement,
    placement: &core::Placement,
) -> Result<(), String> {
    let output_directory = context.output_directory;
    let board_id = context.board_id;
    if output_directory.exists() {
        return Err(format!(
            "output directory {} already exists",
            output_directory.display()
        ));
    }
    copy_directory_tree(context.source_directory, output_directory)?;
    let output_board = output_directory.join(format!("{board_id}.kicad_pcb"));
    fs::write(&output_board, format!("{}\n", encode(pcb)))
        .map_err(|error| format!("failed to write {}: {error}", output_board.display()))?;
    let html = core::view::playback_html(
        &lowered.problem,
        &placement.frames,
        &format!("{board_id} placement"),
    );
    let html_path = output_directory.join("placement.html");
    fs::write(&html_path, html)
        .map_err(|error| format!("failed to write {}: {error}", html_path.display()))?;
    Ok(())
}

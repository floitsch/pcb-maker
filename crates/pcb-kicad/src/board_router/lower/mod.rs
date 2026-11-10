// Copyright (C) 2026 Toit contributors.

//! Lowering a parsed KiCad board to the core router's board: layers,
//! nets, pads, obstacles, pours and rule areas.
//!
//! `lower_board` runs the stages in the order their objects reach the
//! router's board, which the router's result depends on:
//!
//! - `Context::new` (context.rs) reads the board's net names, layers,
//!   outline, planned vias in pads and mask rules: what every stage reads.
//! - `Builder::board_items` lowers the board file's items in their order:
//!   footprints (footprint.rs: rule areas, graphics, texts, then each pad,
//!   pad.rs), kept tracks and vias (tracks.rs), graphics (graphics.rs) and
//!   rule areas (areas.rs). What needs every pad's net first is queued.
//! - Then, once every pad's net is known: pads of one item merged into one
//!   terminal (merge.rs), the queued copper graphics (graphics.rs) and
//!   mask openings (mask.rs), cutouts and keep-whole regions (areas.rs),
//!   the planes (planes.rs), the prepared fan-outs (fanout.rs), and the
//!   board assembled and split by sub-board (`Builder::finish`,
//!   `split_sub_board_nets`).

use super::*;
use pcb_router as core;

mod areas;
mod context;
mod exposed_pad;
mod fanout;
mod footprint;
mod graphics;
mod layers;
mod mask;
mod merge;
mod pad;
mod pin_rows;
mod planes;
mod tracks;
mod warnings;

use self::{context::*, graphics::*, layers::*, mask::*, planes::limit_exclusive_planes};
pub(crate) use self::{
    exposed_pad::{PadBox, exposed_pad},
    layers::LayerTable,
    pin_rows::footprint_pin_row_gaps,
    warnings::input_warnings,
};

pub(super) fn shape(geometry: &ObstacleGeometry) -> core::Shape {
    geometry.router_shape()
}

pub(crate) struct Lowered {
    pub board: core::Board,
    /// The board file's expressions (`expression_count`): its lowering's
    /// work is `lowering_work_of` them.
    pub expressions: u64,
    /// The prepared fan-outs' plan work (`bga_fanout::plan`), in
    /// expressions of the same cost: charged with the lowering.
    pub fanout_work: u64,
}

static LOWER_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static LOWER_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The lowerings' modelled work in nanoseconds (`lowering_work`).
static LOWER_WORK_NANOS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A lowering's work in nanoseconds of the reference per expression of
/// the board file (an atom or a list: `expression_count`): measured on
/// the expansion tier's boards and the named boards, 162 lowerings at
/// one thread in each of two runs, the median 26 and 19 ns (10 % to
/// 90 %: 13 to 55 ns), 0.5 to 22 ms a lowering. Route mode's ladder
/// charges it with the attempt that lowers (docs/router.md, "All the
/// work charged").
pub(crate) const LOWERING_EXPRESSION_NANOS: f64 = 26.0;

/// The expressions (atoms and lists) of a parsed board file.
pub(crate) fn expression_count(pcb: &Expr) -> u64 {
    // A stack of the lists still to count, not recursion; atoms are
    // counted with their list.
    let (mut count, mut pending) = (1, vec![pcb]);
    while let Some(list) = pending.pop() {
        let items = list.children();
        count += items.len() as u64;
        pending.extend(items.iter().filter(|item| matches!(item, Expr::List(_))));
    }
    count
}

/// The work of one lowering of `pcb`, modelled, in seconds at this
/// process's rate (the reference's nanoseconds scaled as the pour
/// model's are).
pub(crate) fn lowering_work(pcb: &Expr) -> f64 {
    lowering_work_of(expression_count(pcb))
}

/// `lowering_work` of a board file of `expressions` expressions.
pub(crate) fn lowering_work_of(expressions: u64) -> f64 {
    expressions as f64 * LOWERING_EXPRESSION_NANOS / 1.0e9 * core::router::reference_time_scale()
}

/// The modelled work of every lowering of this process so far, seconds.
pub(crate) fn lowering_work_total() -> f64 {
    LOWER_WORK_NANOS.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1.0e9
}

/// Calls of `lower` in this process and their seconds (summed over
/// threads), for the board check's line.
pub(crate) fn lowering_totals() -> (u64, f64) {
    use std::sync::atomic::Ordering::Relaxed;
    (
        LOWER_CALLS.load(Relaxed),
        LOWER_NANOS.load(Relaxed) as f64 / 1.0e9,
    )
}

pub(crate) fn lower(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    use std::sync::atomic::Ordering::Relaxed;
    let started = std::time::Instant::now();
    let lowered = lower_board(pcb, config, connect_pours);
    LOWER_CALLS.fetch_add(1, Relaxed);
    LOWER_NANOS.fetch_add(started.elapsed().as_nanos() as u64, Relaxed);
    let expressions =
        expression_count(pcb) + lowered.as_ref().map_or(0, |lowered| lowered.fanout_work);
    LOWER_WORK_NANOS.fetch_add((lowering_work_of(expressions) * 1.0e9) as u64, Relaxed);
    // What the lowering refuses is the board's fault (a form it cannot
    // read, a layer stack it cannot hold), as is a board the router's maps
    // cannot hold (its nets in 16 bits, its layers in 32): an input error,
    // not an internal one.
    let mut lowered = lowered.map_err(crate::verdict::input_error)?;
    lowered.expressions = expressions;
    if std::env::var_os("PCB_ROUTER_BUILD_TRACE").is_some() {
        let board = &lowered.board;
        eprintln!(
            "lowering trace: expressions {} obstacles {} nets {} terminals {} planes {} plane_vertices {} seconds {:.5}",
            expressions,
            board.obstacles.len(),
            board.nets.len(),
            board
                .nets
                .iter()
                .map(|net| net.terminals.len())
                .sum::<usize>(),
            board.planes.len(),
            board
                .planes
                .iter()
                .map(|plane| plane.polygon.len())
                .sum::<usize>(),
            started.elapsed().as_secs_f64()
        );
    }
    core::grid::check_capacity(&lowered.board).map_err(crate::verdict::input_error)?;
    // A lattice the memory cannot hold, before any map is allocated.
    check_lattice_fits(&lowered.board, config)?;
    Ok(lowered)
}

/// The router's board of `pcb` under `config`: the stages in the order
/// their objects reach the board (the module's doc).
fn lower_board(
    pcb: &Expr,
    config: &KiCadBoardRouterConfig,
    connect_pours: bool,
) -> Result<Lowered, String> {
    let ctx = Context::new(pcb, config, connect_pours)?;
    let mut builder = Builder::default();
    builder.board_items(&ctx)?;
    builder.merge_pads();
    builder.join_padstack_copper();
    builder.place_copper_graphics();
    let pours = pours(pcb, &ctx.layers)?;
    builder.place_mask_openings(&ctx, &pours);
    builder.board_cutouts(&ctx);
    builder.keep_whole_keepouts();
    let mut planes = builder.planes(&ctx, &pours);
    limit_exclusive_planes(&ctx, &mut planes);
    if builder.classes.is_empty() {
        return Err("the board has no routable connections".into());
    }
    let via_spans = builder.via_spans(&ctx)?;
    let fanout_work = builder.place_fanouts(&ctx, &via_spans, &planes);
    let mut lowered = builder.finish(&ctx, planes, via_spans, fanout_work);
    split_sub_board_nets(&ctx, &mut lowered.board);
    Ok(lowered)
}

impl Builder {
    /// Every item of the board file, in its order: footprints, kept tracks
    /// and vias, graphics and rule areas. What needs every pad's net first
    /// (board copper graphics, mask openings, keep-whole regions) is queued.
    fn board_items(&mut self, ctx: &Context) -> Result<(), String> {
        for item in ctx.pcb.children() {
            match item.head() {
                Some("footprint") => self.footprint(ctx, item)?,
                Some("segment") => self.existing_segment(ctx, item)?,
                Some("via") => self.existing_via(ctx, item)?,
                Some("arc") => self.existing_arc(ctx, item)?,
                // Board graphics: copper on the copper layers they name
                // (KiCad 9 draws them on several layers at once, with a net:
                // Sisu's GND rectangles on F.Cu and F.Mask), openings on the
                // mask layers. Graphics on a mask layer open the mask: copper
                // of two nets under one opening is a solder bridge (KiCad's
                // solder_mask_bridge), so routed copper stays out from under
                // them on that side.
                Some("dimension" | "gr_dimension" | "gr_text_box") => self.annotation(ctx, item)?,
                Some("gr_text" | "gr_line" | "gr_rect" | "gr_arc" | "gr_circle" | "gr_poly") => {
                    self.board_graphic(ctx, item)?
                }
                Some("zone") if is_rule_area(item) => self.board_rule_area(ctx, item)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// Blind, buried and micro vias (`blind_vias`): the spans the board
    /// allows; a micro via's own size within every class's via.
    fn via_spans(&self, ctx: &Context) -> Result<Vec<core::ViaSpan>, String> {
        let class_via = self
            .classes
            .iter()
            .fold([f64::INFINITY; 2], |least, class| {
                [
                    least[0].min(class.via_diameter),
                    least[1].min(class.via_drill),
                ]
            });
        let (via_spans, _) = allowed_spans(ctx.config, &ctx.layers, class_via)?;
        Ok(via_spans)
    }

    /// The router's board of everything added, with `planes`, `via_spans`
    /// and the fan-outs' plan work.
    fn finish(
        self,
        ctx: &Context,
        planes: Vec<core::Plane>,
        via_spans: Vec<core::ViaSpan>,
        fanout_work: u64,
    ) -> Lowered {
        let Builder {
            classes,
            nets,
            obstacles,
            mut solid_pads,
            mut isolated_pads,
            fill_passes,
            solid_pad_obstacles,
            isolated_pad_obstacles,
            mut no_via_pads,
            pin_row_gaps,
            pad_spokes,
            merged_pads,
            ..
        } = self;
        let config = ctx.config;
        Lowered {
            board: core::Board {
                layer_count: ctx.layers.len(),
                neck_width: config
                    .neck_width_mm
                    .unwrap_or_else(|| {
                        classes
                            .iter()
                            .map(|class| class.trace_width)
                            .fold(f64::INFINITY, f64::min)
                    })
                    .max(config.width_floor()),
                outline: ctx.loops.outline.clone(),
                // Arcs are flattened to chords inside the true curve; the
                // chord error is added so copper stays clear of the real edge.
                edge_clearance: config.edge_clearance_mm + outline::ARC_TOLERANCE,
                hole_clearance: config.hole_clearance_mm,
                hole_to_hole: config.hole_to_hole_clearance_mm,
                classes,
                obstacles,
                nets,
                planes,
                solid_pads: {
                    solid_pads.sort();
                    solid_pads
                },
                isolated_pads: {
                    isolated_pads.sort();
                    isolated_pads
                },
                // In obstacle order already.
                solid_pad_obstacles,
                isolated_pad_obstacles,
                pad_spokes,
                merged_pads,
                no_via_pads: {
                    no_via_pads.sort_unstable();
                    no_via_pads
                },
                fill_passes,
                pin_row_gaps,
                terminal_copper: Vec::new(),
                attachment: Default::default(),
                via_spans,
            },
            expressions: 0,
            fanout_work,
        }
    }
}

/// A panel of different boards: a net per sub-board where no copper joins
/// them (`sub_boards::split_router_nets`). A panel's cells the placement
/// found (`sub_boards::note_cells`): the nets on several of them by design,
/// a net per cell.
fn split_sub_board_nets(ctx: &Context, board: &mut core::Board) {
    let pieces = crate::sub_boards::Pieces::of_loops_cached(
        &ctx.loops.piece_loops,
        &ctx.loops.cutouts,
        &crate::sub_boards::Cuts::of_board(ctx.pcb),
    )
    .map(|pieces| (pieces, None))
    .or_else(|| {
        crate::sub_boards::noted_cells().map(|(cells, designed)| {
            let only: BTreeSet<String> = designed
                .iter()
                .map(|net| ctx.net_names.normalize(net).to_string())
                .collect();
            (std::sync::Arc::new(cells), Some(only))
        })
    });
    if ctx.config.sub_board_nets != Some(false)
        && let Some((pieces, only)) = pieces
    {
        let split = crate::sub_boards::split_router_nets(board, &pieces, only.as_ref());
        if std::env::var_os("PCB_ROUTER_SUB_BOARD_DEBUG").is_some() {
            eprintln!(
                "sub-boards: {} pieces, {} nets split ({} more nets), kept whole: {:?}",
                pieces.count(),
                split.nets.len(),
                split.nets.iter().map(|(_, count)| count - 1).sum::<usize>(),
                split.joined
            );
        }
    }
}

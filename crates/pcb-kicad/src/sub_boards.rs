// Copyright (C) 2026 Toit contributors.

//! Sub-boards of a panel, for the placer: which sub-board each part
//! belongs to (`detect_for_placer`: from the parts that stay where they
//! are, the reference board's places (`--reference`), and the nets no
//! evidence splits), the keepouts that hold each part on its board, and
//! `describe_sub_boards`. The outline's pieces, the router's net split and
//! KiCad's check judged per sub-board are `outline_pieces`
//! (docs/placer.md, "Sub-boards in a panel").

use super::*;
use crate::kicad_sexpr::{Expr, form_at};
use crate::outline_pieces::*;

/// The nets on several sub-boards by design: those the placement split
/// (pins of parts whose board is known on two), and those with pads of
/// two footprints of one reference (a reference names one part per
/// board: two are two boards' parts, the 4in1 panel's two Q6 on
/// `Net-(Q6-G)`), by KiCad's report names.
pub(crate) fn designed_nets(
    pcb: &Expr,
    sub: &PlacerSubBoards,
    net_names: &[String],
) -> BTreeSet<String> {
    // The placer names a net as a connection (`NetNames::normalize`: the
    // hierarchical slash dropped, `/ESC1/GHA` as `ESC1/GHA`); KiCad's
    // report by the pads' own names.
    let names = crate::kicad_nets::NetNames::of(pcb);
    let codes = crate::kicad_nets::net_codes(pcb);
    let split: BTreeSet<&str> = sub
        .split
        .iter()
        .filter_map(|net| net_names.get(*net))
        .map(String::as_str)
        .collect();
    let mut nets: BTreeSet<String> = BTreeSet::new();
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        for pad in footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
        {
            if let Some(net) = crate::kicad_nets::item_net(pad, &codes)
                && split.contains(names.normalize(net))
            {
                nets.insert(net.to_string());
            }
        }
    }
    nets.extend(repeated_reference_nets(pcb));
    nets
}

/// `pcb-maker sub-boards-kicad-board`: a board's pieces without its tabs
/// and the nets its sub-boards split, as JSON (research; the census of
/// docs/placer.md "Sub-boards in a panel").
pub fn describe_sub_boards(board: &Path) -> Result<serde_json::Value, String> {
    let source = fs::read_to_string(board)
        .map_err(|error| format!("failed to read {}: {error}", board.display()))?;
    let pcb = crate::canonical_order::parse_board(&source)?;
    let loops = outline::board_loops(&pcb)?;
    let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
    let cuts = Cuts::of_board(&pcb);
    let Some(pieces) = Pieces::of_loops(&loops.piece_loops, &loops.cutouts, &cuts) else {
        return Ok(serde_json::json!({
            "pieces": 1,
            "outline_loops": loops.piece_loops.len(),
            "mouse_bite_rows": cuts.rows,
            "v_cut_lines": cuts.lines.len(),
        }));
    };
    let split = board_split_nets(&pcb, &pieces);
    // The parts with pins on each piece, by the board's own places: a
    // piece without any is a frame or rail.
    let codes = crate::kicad_nets::net_codes(&pcb);
    let mut parts = vec![0usize; pieces.count()];
    for footprint in pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
    {
        let pinned = footprint
            .children()
            .iter()
            .filter(|child| child.head() == Some("pad"))
            .any(|pad| crate::kicad_nets::item_net(pad, &codes).is_some());
        if pinned
            && let Ok(at) = form_at(footprint)
            && let Some(piece) = pieces.piece_at([at[0], at[1]])
        {
            parts[piece] += 1;
        }
    }
    let occupied: BTreeSet<usize> = (0..pieces.count())
        .filter(|piece| parts[*piece] > 0)
        .collect();
    let frames = pieces.frames(&occupied);
    let keepouts = pieces.keepouts(&frames, TAB_MARGIN_MM);
    Ok(serde_json::json!({
        "pieces": pieces.count(),
        "outline_loops": loops.piece_loops.len(),
        "cut_by": pieces.cut_by,
        "mouse_bite_rows": cuts.rows,
        "v_cut_lines": cuts.lines.len(),
        "boxes": pieces.boxes.iter().map(round).collect::<Vec<_>>(),
        "areas_mm2": pieces.areas.iter().map(|area| area.round()).collect::<Vec<_>>(),
        "parts": parts,
        "frames": frames,
        "tabs": pieces.tabs.iter().map(round).collect::<Vec<_>>(),
        "keepouts": keepouts.len(),
        "kept_clear_mm2": keepouts.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum::<f64>().round(),
        "split_nets": split.len(),
        "split": split,
    }))
}

/// The outline's pieces as `describe-kicad-board` reports them
/// (`sub_boards`), None for a board of one piece: the pieces, what cut
/// them, the tabs, the parts with pins on each by their places (a part
/// sharing its origin with another, a stack, counts on none:
/// `parts_assigned` 0 for a netlist import's stack), the frames (pieces
/// no part lies on, or where none does, by their shape:
/// `Pieces::outline_frames`) and the boards (the other pieces).
pub(crate) fn describe_pieces(pcb: &Expr) -> Result<Option<KiCadDescribedPieces>, String> {
    let loops = outline::board_loops(pcb)?;
    let Some(pieces) = Pieces::of_loops(&loops.piece_loops, &loops.cutouts, &Cuts::of_board(pcb))
    else {
        return Ok(None);
    };
    let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
    let codes = crate::kicad_nets::net_codes(pcb);
    // The parts with pins by their places; a stack (parts sharing one
    // origin, as an import or a task leaves them) tells no board.
    let places: Vec<[f64; 2]> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .filter(|footprint| {
            footprint
                .children()
                .iter()
                .filter(|child| child.head() == Some("pad"))
                .any(|pad| crate::kicad_nets::item_net(pad, &codes).is_some())
        })
        .filter_map(|footprint| form_at(footprint).ok().map(|at| [at[0], at[1]]))
        .collect();
    let key = |at: &[f64; 2]| {
        (
            (at[0] * 100.0).round() as i64,
            (at[1] * 100.0).round() as i64,
        )
    };
    let mut seen: BTreeMap<(i64, i64), usize> = BTreeMap::new();
    for at in &places {
        *seen.entry(key(at)).or_default() += 1;
    }
    let mut parts = vec![0usize; pieces.count()];
    for at in places.iter().filter(|at| seen[&key(at)] == 1) {
        if let Some(piece) = pieces.piece_at(*at) {
            parts[piece] += 1;
        }
    }
    let occupied: BTreeSet<usize> = (0..pieces.count())
        .filter(|piece| parts[*piece] > 0)
        .collect();
    let frames = if occupied.is_empty() {
        pieces.outline_frames()
    } else {
        pieces.frames(&occupied)
    };
    Ok(Some(KiCadDescribedPieces {
        pieces: pieces.count(),
        cut_by: pieces.cut_by.iter().map(|kind| kind.to_string()).collect(),
        tabs: pieces.tabs.iter().map(round).collect(),
        frames: pieces
            .boxes
            .iter()
            .zip(&frames)
            .filter(|(_, frame)| **frame)
            .map(|(b, _)| round(b))
            .collect(),
        frames_by: if occupied.is_empty() {
            "shape"
        } else {
            "parts"
        },
        parts_assigned: parts.iter().sum(),
        sub_boards: pieces
            .boxes
            .iter()
            .zip(&frames)
            .zip(&parts)
            .filter(|((_, frame), _)| !**frame)
            .map(|((b, _), parts)| KiCadDescribedSubBoard {
                region: round(b),
                parts: *parts,
            })
            .collect(),
    }))
}

/// A sub-board as `describe-kicad-board` reports it: its box and the
/// parts with pins whose origin lies on it (a stack off every piece
/// counts on none). Which of them the placement moves is the place
/// check's (`sub_boards.sub_boards[].movable` of board-placer.json): a
/// description knows no constraints (trial 5's SmartSpin2k read `movable:
/// 0` on every board, with 630 parts to place).
#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescribedSubBoard {
    pub region: [f64; 4],
    pub parts: usize,
}

/// `describe-kicad-board`'s `sub_boards` (`describe_pieces`).
#[derive(Clone, Debug, Serialize)]
pub struct KiCadDescribedPieces {
    /// The outline's pieces without its tabs, and what cut them
    /// (`narrow outline`, `mouse bites`, `V-cuts`).
    pub pieces: usize,
    pub cut_by: Vec<String>,
    pub tabs: Vec<[f64; 4]>,
    /// The pieces that are a frame or rails: by the parts (no part with
    /// pins lies on them) or, where no part lies on any piece (a netlist
    /// import's stack), by their shape (`frames_by`: `parts`, `shape`).
    pub frames: Vec<[f64; 4]>,
    pub frames_by: &'static str,
    /// Parts with pins whose origin lies on a piece: 0 where nothing says
    /// which parts go on which board (give `--reference` or a region per
    /// board).
    pub parts_assigned: usize,
    /// The other pieces, the boards: each one's box (a region for its
    /// parts) and the parts on it.
    pub sub_boards: Vec<KiCadDescribedSubBoard>,
}

/// The sub-boards the placer found (`detect_for_placer`).
#[derive(Clone, Debug)]
pub(crate) struct PlacerSubBoards {
    pub pieces: std::sync::Arc<Pieces>,
    /// Per footprint (the problem's first components): its sub-board,
    /// None where nothing says which.
    pub part_piece: Vec<Option<usize>>,
    /// The problem's nets with pins on two sub-boards or more.
    pub split: Vec<usize>,
    /// What placed the parts on their sub-boards: `fixed parts`,
    /// `reference board`, `placed input`, `nets`.
    pub from: Vec<&'static str>,
    /// Groups of parts nets join that the evidence puts on two sub-boards
    /// (left without one).
    pub conflicts: usize,
    /// Per piece: a frame or rail (`Pieces::frames`): no part with pins
    /// lies on it by the fixed parts, the reference board or the placed
    /// input; no part may.
    pub frames: Vec<bool>,
    /// Per footprint: a connector that may reach over the frames and tabs
    /// (`overhang_connector`).
    pub connectors: Vec<bool>,
    /// Per footprint: its place on the reference board (the designer's),
    /// where there is one.
    pub designer: Vec<Option<pcb_placer::Pose>>,
}

/// Connector families by footprint name (lower case, anywhere in the
/// name): their bodies reach past their pads (a receptacle's shell, a
/// right-angle header's housing).
const CONNECTOR_NAMES: &[&str] = &[
    "connector",
    "conn_",
    "jst",
    "molex",
    "hirose",
    "pinheader",
    "pin_header",
    "pinsocket",
    "pin_socket",
    "header",
    "socket",
    "receptacle",
    "type-c",
    "typec",
    "micro_sd",
    "microsd",
    "sim_card",
];

/// Whether a footprint is a connector that may reach over a panel's frame
/// or tab where its copper stays on its board
/// (`pcb_placer::constraints::FramePass`): a plug-in connector by its
/// footprint name (`board_placer::edge_connector`: USB, SMA, jacks,
/// card edges, terminal blocks), a connector family's name
/// (`CONNECTOR_NAMES`: JST, Molex, Hirose, pin headers and sockets), the
/// USB-C pads (A1, A12, B1 and B12), or a connector's reference (J, P,
/// CN, CON, USB, X; not a crystal) whose courtyard reaches past its pads
/// on one side (`connector_mouth`).
pub(crate) fn overhang_connector(footprint: &Expr) -> bool {
    let name = footprint
        .children()
        .get(1)
        .and_then(Expr::atom)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if crate::board_placer::edge_connector(&name)
        || CONNECTOR_NAMES.iter().any(|key| name.contains(key))
    {
        return true;
    }
    let pads: BTreeSet<&str> = footprint
        .children()
        .iter()
        .filter(|child| child.head() == Some("pad"))
        .filter_map(|pad| pad.children().get(1).and_then(Expr::atom))
        .collect();
    if ["A1", "A12", "B1", "B12"]
        .iter()
        .all(|pad| pads.contains(pad))
    {
        return true;
    }
    let reference = crate::kicad_nets::footprint_reference(footprint).unwrap_or_default();
    let prefix: String = reference
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    matches!(prefix.as_str(), "J" | "P" | "CN" | "CON" | "USB" | "X")
        && !name.contains("crystal")
        && matches!(crate::board_placer::connector_mouth(footprint), Ok(Some(_)))
}

/// The share of the movable parts that may be without a sub-board for
/// the placement to hold the others in theirs and split the nets.
pub(crate) const MOST_UNASSIGNED_SHARE: f64 = 0.1;

fn body_box(component: &pcb_placer::Component, pose: pcb_placer::Pose) -> [f64; 4] {
    let (center, half) = (component.center(pose), component.half_extent(pose.angle));
    [
        center[0] - half[0],
        center[1] - half[1],
        center[0] + half[0],
        center[1] + half[1],
    ]
}

/// Which sub-board each part belongs to (module comment), on a board whose
/// outline has two pieces or more without its tabs (else None):
///
/// 1. the parts that stay where they are (fixed, locked, the board's
///    marks among the footprints) lie on their piece;
/// 2. with a reference board (`reference`), each footprint whose uuid it
///    holds lies on the piece of its place there;
/// 3. an input that looks placed (nine in ten movable parts on a piece,
///    nine in ten at a place of their own: not a stack) gives each
///    movable part its piece;
/// 4. a net with pins of such parts on two pieces is split; the others
///    join their parts, and a group so joined takes the piece of its
///    placed parts where they agree (none where they do not: a conflict).
pub(crate) fn detect_for_placer(
    pcb: &Expr,
    problem: &pcb_placer::Problem,
    footprints: usize,
    reference: Option<&Expr>,
) -> Result<Option<PlacerSubBoards>, String> {
    let loops = outline::board_loops(pcb)?;
    let Some(pieces) =
        Pieces::of_loops_cached(&loops.piece_loops, &loops.cutouts, &Cuts::of_board(pcb))
    else {
        return Ok(None);
    };
    let footprints = footprints.min(problem.components.len());
    let footprint_items: Vec<&Expr> = pcb
        .children()
        .iter()
        .filter(|item| item.head() == Some("footprint"))
        .collect();
    let uuids: Vec<Option<String>> = footprint_items
        .iter()
        .map(|item| crate::kicad_nets::footprint_uuid(item))
        .collect();
    let mut designer_places: BTreeMap<String, [f64; 3]> = BTreeMap::new();
    let mut part_piece: Vec<Option<usize>> = vec![None; footprints];
    let mut from: BTreeSet<&'static str> = BTreeSet::new();
    let with_pins = |part: usize| !problem.components[part].pins.is_empty();
    for (part, place) in part_piece.iter_mut().enumerate() {
        let component = &problem.components[part];
        if component.fixed
            && let Some(Some(piece)) = pieces.piece_of_box(body_box(component, problem.poses[part]))
        {
            *place = Some(piece);
            if with_pins(part) {
                from.insert("fixed parts");
            }
        }
    }
    if let Some(reference) = reference {
        designer_places = reference
            .children()
            .iter()
            .filter(|item| item.head() == Some("footprint"))
            .filter_map(|item| {
                Some((
                    crate::kicad_nets::footprint_uuid(item)?,
                    form_at(item).ok()?,
                ))
            })
            .collect();
        let places = &designer_places;
        for (part, place) in part_piece.iter_mut().enumerate() {
            if place.is_none()
                && let Some(Some(uuid)) = uuids.get(part)
                && let Some(at) = places.get(uuid)
                && let Some(piece) = pieces.piece_at([at[0], at[1]])
            {
                *place = Some(piece);
                from.insert("reference board");
            }
        }
    }
    let movable: Vec<usize> = (0..footprints)
        .filter(|part| !problem.components[*part].fixed && with_pins(*part))
        .collect();
    if !movable.is_empty() {
        let on_piece = movable
            .iter()
            .filter(|part| pieces.piece_at(problem.poses[**part].position).is_some())
            .count();
        let mut seen: BTreeMap<(i64, i64), usize> = BTreeMap::new();
        for part in &movable {
            let at = problem.poses[*part].position;
            *seen
                .entry((
                    (at[0] * 100.0).round() as i64,
                    (at[1] * 100.0).round() as i64,
                ))
                .or_default() += 1;
        }
        let alone = movable
            .iter()
            .filter(|part| {
                let at = problem.poses[**part].position;
                seen[&(
                    (at[0] * 100.0).round() as i64,
                    (at[1] * 100.0).round() as i64,
                )] == 1
            })
            .count();
        if on_piece * 10 >= movable.len() * 9 && alone * 10 >= movable.len() * 9 {
            for part in &movable {
                if part_piece[*part].is_none()
                    && let Some(piece) = pieces.piece_at(problem.poses[*part].position)
                {
                    part_piece[*part] = Some(piece);
                    from.insert("placed input");
                }
            }
        }
    }
    // The frames: pieces no part with pins lies on by that evidence.
    let occupied: BTreeSet<usize> = part_piece
        .iter()
        .enumerate()
        .filter(|(part, _)| with_pins(*part))
        .filter_map(|(_, piece)| *piece)
        .collect();
    let frames = pieces.frames(&occupied);
    // The nets: on two pieces by the parts placed so far, split.
    let net_count = problem.net_weights.len();
    let mut on: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); net_count];
    for (part, place) in part_piece.iter().enumerate() {
        if let Some(piece) = *place {
            for pin in &problem.components[part].pins {
                on[pin.net].insert(piece);
            }
        }
    }
    let split: Vec<usize> = (0..net_count).filter(|net| on[*net].len() >= 2).collect();
    let split_set: BTreeSet<usize> = split.iter().copied().collect();
    // The other nets join their parts.
    let mut parent: Vec<usize> = (0..footprints).collect();
    fn root(parent: &mut [usize], mut part: usize) -> usize {
        while parent[part] != part {
            parent[part] = parent[parent[part]];
            part = parent[part];
        }
        part
    }
    let mut first_on_net: BTreeMap<usize, usize> = BTreeMap::new();
    for part in 0..footprints {
        for pin in &problem.components[part].pins {
            if split_set.contains(&pin.net) {
                continue;
            }
            match first_on_net.get(&pin.net) {
                Some(&other) => {
                    let (a, b) = (root(&mut parent, part), root(&mut parent, other));
                    parent[a] = b;
                }
                None => {
                    first_on_net.insert(pin.net, part);
                }
            }
        }
    }
    let mut group_pieces: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (part, place) in part_piece.iter().enumerate() {
        if let Some(piece) = *place {
            let group = root(&mut parent, part);
            group_pieces.entry(group).or_default().insert(piece);
        }
    }
    let conflicts = group_pieces
        .values()
        .filter(|pieces| pieces.len() > 1)
        .count();
    let mut by_nets = false;
    for (part, place) in part_piece.iter_mut().enumerate() {
        if place.is_none() && with_pins(part) {
            let group = root(&mut parent, part);
            if let Some(pieces) = group_pieces.get(&group)
                && pieces.len() == 1
            {
                *place = pieces.iter().next().copied();
                by_nets = true;
            }
        }
    }
    if by_nets {
        from.insert("nets");
    }
    let order = ["fixed parts", "reference board", "placed input", "nets"];
    let connectors: Vec<bool> = footprint_items
        .iter()
        .take(footprints)
        .enumerate()
        .map(|(part, item)| with_pins(part) && overhang_connector(item))
        .collect();
    let designer: Vec<Option<pcb_placer::Pose>> = (0..footprints)
        .map(|part| {
            let uuid = uuids.get(part)?.as_ref()?;
            let at = designer_places.get(uuid)?;
            Some(pcb_placer::Pose {
                position: [at[0], at[1]],
                angle: at[2],
            })
        })
        .collect();
    Ok(Some(PlacerSubBoards {
        pieces,
        part_piece,
        split,
        from: order
            .into_iter()
            .filter(|kind| from.contains(kind))
            .collect(),
        conflicts,
        frames,
        connectors,
        designer,
    }))
}

impl PlacerSubBoards {
    /// Whether some pieces are a frame or rails (`frames`).
    pub(crate) fn has_frames(&self) -> bool {
        self.frames.iter().any(|frame| *frame)
    }

    /// The sub-boards that hold parts with pins: a panel of different
    /// boards has two or more.
    pub(crate) fn count(&self, problem: &pcb_placer::Problem) -> usize {
        self.part_piece
            .iter()
            .enumerate()
            .filter(|(part, _)| !problem.components[*part].pins.is_empty())
            .filter_map(|(_, piece)| *piece)
            .collect::<BTreeSet<usize>>()
            .len()
    }

    /// The movable parts with pins and those of them without a sub-board.
    pub(crate) fn unassigned(&self, problem: &pcb_placer::Problem) -> (usize, Vec<usize>) {
        let movable: Vec<usize> = (0..self.part_piece.len())
            .filter(|part| {
                let component = &problem.components[*part];
                !component.fixed && !component.pins.is_empty()
            })
            .collect();
        let unassigned = movable
            .iter()
            .copied()
            .filter(|part| self.part_piece[*part].is_none())
            .collect();
        (movable.len(), unassigned)
    }
}

/// What `apply_to_placer` changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PlacerApplied {
    /// Parts held in their sub-board's region.
    pub held: usize,
    /// Nets of the problem split, and the nets added for them.
    pub nets_split: usize,
    pub nets_added: usize,
    /// Keepouts added (boxes over the tabs and the frames,
    /// `Pieces::keepouts`), and their area (mm², overlaps counted once).
    pub tabs: usize,
    pub kept_clear_mm2: f64,
    /// Movable parts that may reach over the keepouts
    /// (`pcb_placer::constraints::FramePass`): connectors, and parts the
    /// designer's placement put into them (by how far).
    pub connectors: usize,
    pub designer_overhangs: usize,
    /// How far the keepouts reach beyond the tabs and frames (mm):
    /// `TAB_MARGIN_MM`; None where nothing is kept off (a panel of copies
    /// too small for the margin, `MOST_COPY_KEPT_SHARE`).
    pub margin_mm: Option<f64>,
    /// On a panel of copies: the share of the first copy's surroundings
    /// the keepouts grown by `TAB_MARGIN_MM` take.
    pub copy_kept_share: f64,
}

/// On a panel of copies, the keepouts grown by `TAB_MARGIN_MM` take at
/// most this share of the first copy's surroundings; past it nothing is
/// kept off (OpenRX's copies: 0.40; the 0xCB panel 0.02,
/// the pcb-panel 0.07, SmartSpin2k 0.15).
const MOST_COPY_KEPT_SHARE: f64 = 0.3;

/// How far the body `b` reaches into the keepouts: the least that, taken
/// off every side, leaves it clear of each (0 where it meets none).
fn depth_into(b: [f64; 4], keepouts: &[[f64; 4]]) -> f64 {
    keepouts
        .iter()
        .filter(|k| b[0] < k[2] && k[0] < b[2] && b[1] < k[3] && k[1] < b[3])
        .map(|k| {
            let x = (b[2] - k[0]).min(k[2] - b[0]);
            let y = (b[3] - k[1]).min(k[3] - b[1]);
            x.min(y).max(0.0)
        })
        .fold(0.0, f64::max)
}

/// Keeps every part off the panel's tabs and frames: each box of
/// `Pieces::keepouts` (the tabs and the frames grown by `TAB_MARGIN_MM`)
/// a fixed obstacle of no side, but for the parts that may reach over
/// them (`pcb_placer::constraints::Frames`):
///
/// - a part the designer's placement (the reference board) put into the
///   keepouts may reach as far into them (its body shrunk by that much
///   keeps off them; a fixed part stays where it is anyway);
/// - on a panel (a board with a frame or rails, or a panel of copies:
///   `panel`), a connector (`overhang_connector`) may reach over them
///   with its body where its pads and copper graphics keep the edge margin
///   off the tabs and frames themselves (`Pieces::copper`): on its own
///   board. A board of two halves joined by tabs and no frame (urchin,
///   woagboard) is one board whose tabs lie inside it: nothing reaches
///   over them.
///
/// The board's outline is not changed: a part still keeps inside it.
///
/// On a panel of copies (`copies`) each copy keeps off every copy's
/// keepouts carried into its frame (`Pieces::copies_keepouts`).
pub(crate) fn keep_off_frames(
    sub: &PlacerSubBoards,
    problem: &mut pcb_placer::Problem,
    references: &mut Vec<String>,
    source_at: &mut Vec<[f64; 3]>,
    applied: &mut PlacerApplied,
    panel: bool,
    copies: Option<&CopyFrames>,
) {
    applied.margin_mm = Some(TAB_MARGIN_MM);
    let (keepouts, copper) = match copies {
        Some(copies) => {
            let grown = sub
                .pieces
                .copies_keepouts(&sub.frames, copies, TAB_MARGIN_MM);
            // Copies too small for the margin (OpenRX's 10.5 x 11.5 mm
            // copies: their parts found no place with it, and kept off
            // the tabs and frames alone they routed worse) keep nothing
            // off, as before.
            let within = copies.within[0];
            let share = grown
                .0
                .iter()
                .map(|b| {
                    let cut = clip_box(*b, within);
                    (cut[2] - cut[0]).max(0.0) * (cut[3] - cut[1]).max(0.0)
                })
                .sum::<f64>()
                / ((within[2] - within[0]) * (within[3] - within[1])).max(1.0e-9);
            applied.copy_kept_share = share;
            if share > MOST_COPY_KEPT_SHARE {
                applied.margin_mm = None;
                return;
            }
            grown
        }
        None => (
            sub.pieces.keepouts(&sub.frames, TAB_MARGIN_MM),
            sub.pieces.copper(&sub.frames),
        ),
    };
    if keepouts.is_empty() {
        return;
    }
    applied.kept_clear_mm2 = keepouts.iter().map(|b| (b[2] - b[0]) * (b[3] - b[1])).sum();
    let mut passes = Vec::new();
    for part in 0..sub.part_piece.len().min(problem.components.len()) {
        let component = &problem.components[part];
        if component.fixed || component.pins.is_empty() {
            continue;
        }
        let overhang = match sub.designer.get(part).copied().flatten() {
            Some(pose) => {
                let depth = depth_into(body_box(component, pose), &keepouts);
                if depth > 0.0 { depth + 1.0e-6 } else { 0.0 }
            }
            None => 0.0,
        };
        let connector = panel && sub.connectors.get(part).copied().unwrap_or(false);
        if overhang > 0.0 || connector {
            applied.connectors += connector as usize;
            applied.designer_overhangs += (overhang > 0.0) as usize;
            passes.push((
                part,
                pcb_placer::constraints::FramePass {
                    overhang,
                    connector,
                },
            ));
        }
    }
    let first = problem.components.len();
    for tab in &keepouts {
        let size = [tab[2] - tab[0], tab[3] - tab[1]];
        problem.components.push(pcb_placer::Component {
            name: "tab".into(),
            body_center: [0.0, 0.0],
            body_size: size,
            round: false,
            halo: 0.0,
            pins: Vec::new(),
            side: pcb_placer::Side::Both,
            fixed: true,
            angle_options: vec![0.0],
            far_side: Vec::new(),
            far_copper: Vec::new(),
            far_open: Default::default(),
            far_courtyard: Vec::new(),
            hollow: Vec::new(),
            tight: None,
            edge_inset: 0.0,
            courtyards: Vec::new(),
            holes_inside: false,
            pads: Vec::new(),
            copper_only: false,
            cutout_outline: Vec::new(),
            tight_hollow: Vec::new(),
            hole_rings: Vec::new(),
            copper_graphics: Vec::new(),
            courtless: Vec::new(),
            slots: Vec::new(),
        });
        problem.poses.push(pcb_placer::Pose {
            position: [(tab[0] + tab[2]) / 2.0, (tab[1] + tab[3]) / 2.0],
            angle: 0.0,
        });
        references.push("tab".into());
        source_at.push([0.0; 3]);
        applied.tabs += 1;
    }
    let frames = &mut problem.constraints.frames;
    frames.keepouts = (first..problem.components.len()).collect();
    if !passes.is_empty() {
        frames.copper = copper;
        frames.passes = passes;
    }
}

/// The sub-boards in the placement problem (a panel: two sub-boards or
/// more, or a board beside its frame or rails):
///
/// - no part's body on a tab or a frame: each box of `Pieces::keepouts`
///   (the tabs grown by `TAB_MARGIN_MM`, the frames) a keepout (an
///   obstacle of the board, as a rule area's), where the parts are held
///   (below) or the board has a frame;
/// - where the parts lie on two sub-boards or more and at most
///   `MOST_UNASSIGNED_SHARE` of the movable parts have none, each movable
///   part held in its sub-board's region (unless a constraint holds it
///   in one already) and started at its centre, and each split net one
///   net per sub-board (the parts without one stay on the first's), its
///   weight as the lowering gives a net of that many pins.
pub(crate) fn apply_to_placer(
    sub: &PlacerSubBoards,
    problem: &mut pcb_placer::Problem,
    references: &mut Vec<String>,
    source_at: &mut Vec<[f64; 3]>,
    poured: &mut Vec<bool>,
) -> PlacerApplied {
    let mut applied = PlacerApplied::default();
    // Most parts' boards unknown: no part is held (tab keepouts alone
    // cost the crowded 4in1 panel's front 70 mm², and 8 parts found no
    // place where 0 to 4 had not), unless the panel has a frame.
    let (movable, unassigned) = sub.unassigned(problem);
    // Held also where no net lies on two sub-boards (the ULK's halves:
    // their nets apart, each part's half told by the nets): a part on
    // the other half is no part of its circuit there (no track crosses
    // a row of mouse bites).
    let hold = sub.count(problem) >= 2
        && unassigned.len() as f64 <= MOST_UNASSIGNED_SHARE * movable as f64;
    if !hold && !sub.has_frames() {
        return applied;
    }
    keep_off_frames(
        sub,
        problem,
        references,
        source_at,
        &mut applied,
        sub.has_frames(),
        None,
    );
    if !hold {
        return applied;
    }
    let held: BTreeSet<usize> = problem
        .constraints
        .regions
        .iter()
        .map(|(part, _)| *part)
        .collect();
    let started: BTreeSet<usize> = problem
        .constraints
        .starts
        .iter()
        .map(|(part, _)| *part)
        .collect();
    for part in 0..sub.part_piece.len() {
        let component = &problem.components[part];
        let Some(piece) = sub.part_piece[part] else {
            continue;
        };
        if component.fixed || component.pins.is_empty() || held.contains(&part) {
            continue;
        }
        let region = sub.pieces.boxes[piece];
        problem.constraints.regions.push((part, region));
        if !started.contains(&part) {
            problem.constraints.starts.push((
                part,
                [(region[0] + region[2]) / 2.0, (region[1] + region[3]) / 2.0],
            ));
        }
        applied.held += 1;
    }
    // The split nets: each sub-board's pins on a net of their own.
    let mut pin_counts = vec![0usize; problem.net_weights.len()];
    for component in &problem.components {
        for pin in &component.pins {
            pin_counts[pin.net] += 1;
        }
    }
    let net_weight = crate::board_placer::net_weight;
    let base = |net: usize, weights: &[f64]| weights[net] / net_weight(pin_counts[net]);
    for &net in &sub.split {
        let mut ids: BTreeMap<usize, usize> = BTreeMap::new();
        let scale = base(net, &problem.net_weights);
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        for part in 0..sub.part_piece.len() {
            let piece = sub.part_piece[part];
            for pin in problem.components[part].pins.iter_mut() {
                if pin.net != net {
                    continue;
                }
                let Some(piece) = piece else {
                    *counts.entry(net).or_default() += 1;
                    continue;
                };
                let id = match ids.get(&piece) {
                    Some(id) => *id,
                    None => {
                        let id = if ids.is_empty() {
                            net
                        } else {
                            problem.net_weights.push(0.0);
                            poured.push(poured.get(net).copied().unwrap_or(false));
                            applied.nets_added += 1;
                            problem.net_weights.len() - 1
                        };
                        ids.insert(piece, id);
                        id
                    }
                };
                pin.net = id;
                *counts.entry(id).or_default() += 1;
            }
        }
        for (id, count) in counts {
            problem.net_weights[id] = scale * net_weight(count);
        }
        applied.nets_split += 1;
    }
    applied
}

/// The sub-boards as `board-placer.json` and the verdict say them
/// (`sub_boards`): present where the outline has two pieces or more
/// without its tabs and parts lie on two of them.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadSubBoardsReport {
    /// How many sub-boards hold parts.
    pub count: usize,
    /// What told the parts' sub-boards: `fixed parts`, `reference board`,
    /// `placed input`, `nets`.
    pub from: Vec<String>,
    /// Nets (by name) whose parts lie on two sub-boards or more: one net
    /// per sub-board to the placer and the router where no copper joins
    /// them.
    pub split_nets: usize,
    /// Movable parts no evidence puts on a sub-board: placed where the
    /// placement put them, the nets they share are the router's per
    /// sub-board where their pads land. The first 20 by label.
    pub unassigned: usize,
    pub unassigned_parts: Vec<String>,
    /// Groups of parts joined by nets that the evidence put on two
    /// sub-boards (their unplaced parts left without one).
    pub conflicts: usize,
    /// Whether the placement held the parts in their sub-boards (at most
    /// a tenth of the movable parts without one), and the tabs (boxes)
    /// it kept parts off.
    pub held: bool,
    pub tabs: Vec<[f64; 4]>,
    /// What cut the outline into pieces: `narrow outline`, `mouse
    /// bites`, `V-cuts`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cut_by: Vec<String>,
    /// The boxes of the pieces that are a frame or rails (no part with
    /// pins lies on them by the fixed parts, the reference or the placed
    /// input): no part is placed there.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<[f64; 4]>,
    /// The area kept free of parts over the tabs (grown by 1.5 mm) and the
    /// frames (mm²); 0 where the placement kept nothing free.
    #[serde(default)]
    pub kept_clear_mm2: f64,
    /// Movable parts that may reach over the frames and tabs: connectors
    /// (their pads and copper on their board) and parts the designer's
    /// placement (the reference board) put into the kept area, as far as
    /// it did.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub overhang_connectors: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub overhang_designer: usize,
    /// A panel of copies (`panel_copies`): the copies are placed as
    /// copies, each in its region, kept off the frames and tabs; their
    /// parts are not held by sub-board.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub copies: bool,
    /// How far the keepouts reach beyond the tabs and frames (mm): 1.5;
    /// null where nothing is kept off: a panel of copies too small for the
    /// margin (the keepouts grown by it would take more than 0.3 of the
    /// first copy's surroundings) is placed as before.
    #[serde(default)]
    pub keepout_margin_mm: Option<f64>,
    /// Each sub-board: its region and parts (with pins), of them the
    /// movable ones.
    pub sub_boards: Vec<KiCadSubBoard>,
    /// The outline's pieces without its tabs (boards and frames alike).
    #[serde(default)]
    pub pieces: usize,
    /// The movable parts with pins, and the parts with pins the evidence
    /// put on a piece: 0 where no part lies on any (a netlist import's
    /// stack without `--reference`): then `sub_boards` lists every piece
    /// but the frames, with no parts, and `frames` are the pieces that
    /// look like a frame or rails by the outline alone; nothing is held
    /// or kept off.
    #[serde(default)]
    pub movable: usize,
    #[serde(default)]
    pub parts_assigned: usize,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct KiCadSubBoard {
    pub region: [f64; 4],
    pub parts: usize,
    pub movable: usize,
}

impl KiCadSubBoardsReport {
    pub(crate) fn of(
        sub: &PlacerSubBoards,
        problem: &pcb_placer::Problem,
        references: &[String],
        names: &[String],
        applied: &PlacerApplied,
    ) -> KiCadSubBoardsReport {
        let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
        let (_, unassigned) = sub.unassigned(problem);
        let mut sub_boards: Vec<KiCadSubBoard> = sub
            .pieces
            .boxes
            .iter()
            .map(|b| KiCadSubBoard {
                region: round(b),
                parts: 0,
                movable: 0,
            })
            .collect();
        for (part, piece) in sub.part_piece.iter().enumerate() {
            if let Some(piece) = piece
                && !problem.components[part].pins.is_empty()
            {
                sub_boards[*piece].parts += 1;
                if !problem.components[part].fixed {
                    sub_boards[*piece].movable += 1;
                }
            }
        }
        sub_boards.retain(|board| board.parts > 0);
        let (movable, _) = sub.unassigned(problem);
        let parts_assigned = sub
            .part_piece
            .iter()
            .enumerate()
            .filter(|(part, piece)| piece.is_some() && !problem.components[*part].pins.is_empty())
            .count();
        KiCadSubBoardsReport {
            pieces: sub.pieces.count(),
            movable,
            parts_assigned,
            count: sub.count(problem),
            from: sub.from.iter().map(|kind| kind.to_string()).collect(),
            split_nets: sub.split.len(),
            unassigned: unassigned.len(),
            unassigned_parts: unassigned
                .iter()
                .take(20)
                .map(|part| {
                    names
                        .get(*part)
                        .or(references.get(*part))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect(),
            conflicts: sub.conflicts,
            held: applied.held > 0,
            tabs: sub.pieces.tabs.iter().map(round).collect(),
            cut_by: sub
                .pieces
                .cut_by
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
            frames: sub
                .pieces
                .boxes
                .iter()
                .zip(&sub.frames)
                .filter(|(_, frame)| **frame)
                .map(|(b, _)| round(b))
                .collect(),
            kept_clear_mm2: (applied.kept_clear_mm2 * 10.0).round() / 10.0,
            overhang_connectors: applied.connectors,
            overhang_designer: applied.designer_overhangs,
            copies: false,
            keepout_margin_mm: applied.margin_mm,
            sub_boards,
        }
    }
}

impl KiCadSubBoardsReport {
    /// The pieces and frames found from the outline alone, where no part
    /// lies on any piece (`parts_assigned` 0: a netlist import's stack
    /// without `--reference`): the pieces but the frames as sub-boards
    /// without parts, the frames by their shape (`Pieces::outline_frames`),
    /// the tabs, every movable part unassigned. The placement is as
    /// without it (nothing held, nothing kept off).
    pub(crate) fn of_outline(
        sub: &PlacerSubBoards,
        problem: &pcb_placer::Problem,
        names: &[String],
    ) -> KiCadSubBoardsReport {
        let round = |b: &[f64; 4]| b.map(|v| (v * 1000.0).round() / 1000.0);
        let (movable, unassigned) = sub.unassigned(problem);
        let frames = sub.pieces.outline_frames();
        KiCadSubBoardsReport {
            count: 0,
            from: Vec::new(),
            split_nets: 0,
            unassigned: unassigned.len(),
            unassigned_parts: unassigned
                .iter()
                .take(20)
                .map(|part| names.get(*part).cloned().unwrap_or_default())
                .collect(),
            conflicts: 0,
            held: false,
            tabs: sub.pieces.tabs.iter().map(round).collect(),
            cut_by: sub
                .pieces
                .cut_by
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
            frames: sub
                .pieces
                .boxes
                .iter()
                .zip(&frames)
                .filter(|(_, frame)| **frame)
                .map(|(b, _)| round(b))
                .collect(),
            kept_clear_mm2: 0.0,
            overhang_connectors: 0,
            overhang_designer: 0,
            copies: false,
            keepout_margin_mm: None,
            sub_boards: sub
                .pieces
                .boxes
                .iter()
                .zip(&frames)
                .filter(|(_, frame)| !**frame)
                .map(|(b, _)| KiCadSubBoard {
                    region: round(b),
                    parts: 0,
                    movable: 0,
                })
                .collect(),
            pieces: sub.pieces.count(),
            movable,
            parts_assigned: 0,
        }
    }

    /// Whether most parts' boards are unknown on an outline of several
    /// pieces: more than a tenth of the movable parts have none, and the
    /// parts tell fewer than two boards (a netlist import's stack without
    /// `--reference`). `boards_mixed` covers two boards or more.
    pub fn boards_unknown(&self) -> bool {
        self.pieces >= 2
            && self.count < 2
            && !self.copies
            && !self.held
            && self.unassigned as f64
                > crate::sub_boards::MOST_UNASSIGNED_SHARE * self.movable as f64
    }
}

/// The looser of two relaxations, field by field (as `panel::loosest`).
fn loosest(a: &pcb_placer::Relaxation, b: &pcb_placer::Relaxation) -> pcb_placer::Relaxation {
    let mut on_marks: Vec<usize> = a.on_marks.iter().chain(&b.on_marks).copied().collect();
    on_marks.sort_unstable();
    on_marks.dedup();
    pcb_placer::Relaxation {
        spacing: a.spacing.min(b.spacing),
        grid: a.grid.min(b.grid),
        halo_scale: a.halo_scale.min(b.halo_scale),
        small_halo_scale: a.small_halo_scale.min(b.small_halo_scale),
        edge_inset: a.edge_inset || b.edge_inset,
        tight: a.tight || b.tight,
        edge_copper: a.edge_copper || b.edge_copper,
        edge_rule: a.edge_rule.min(b.edge_rule),
        courtyard_spacing: a.courtyard_spacing || b.courtyard_spacing,
        pad_clearance: a.pad_clearance || b.pad_clearance,
        halos: a.halos.clone().or_else(|| b.halos.clone()),
        on_marks,
    }
}

/// A part that takes no room and pulls no net (as `panel::make_inert`).
fn make_inert(component: &mut pcb_placer::Component) {
    component.fixed = true;
    component.side = pcb_placer::Side::Neither;
    component.pins.clear();
    component.far_side.clear();
    component.far_copper.clear();
    component.far_courtyard.clear();
    component.hollow.clear();
    component.pads.clear();
    component.courtyards.clear();
    component.tight_hollow.clear();
    component.hole_rings.clear();
    component.copper_graphics.clear();
    component.copper_only = false;
    component.cutout_outline.clear();
    component.halo = 0.0;
}

/// One sub-board's problem: the outline cut to its region, its movable
/// parts (`active`) the only ones placed; the other movable parts take
/// no room and pull no net; fixed parts away from the region keep their
/// bodies but pull no net; the constraints of the active parts only.
/// A panel's frame and tab keepouts (`keep_off_frames`) in a problem of
/// one region (a copy's, a held sub-board's): cut to the region, none
/// outside it. The region's parts are held in it, and the keepouts beyond
/// (the other sub-boards' tabs, the rails), counted whole against the
/// region's area, made every placement level but the last overfull.
pub(crate) fn cut_keepouts(sub: &mut pcb_placer::Problem, region: [f64; 4]) {
    for keepout in sub.constraints.frames.keepouts.clone() {
        let pose = sub.poses[keepout];
        let component = &mut sub.components[keepout];
        let (center, half) = (component.center(pose), component.half_extent(pose.angle));
        let cut = clip_box(
            [
                center[0] - half[0],
                center[1] - half[1],
                center[0] + half[0],
                center[1] + half[1],
            ],
            region,
        );
        if !(cut[0] < cut[2] && cut[1] < cut[3]) {
            make_inert(component);
            continue;
        }
        component.body_center = [0.0, 0.0];
        component.body_size = [cut[2] - cut[0], cut[3] - cut[1]];
        sub.poses[keepout] = pcb_placer::Pose {
            position: [(cut[0] + cut[2]) / 2.0, (cut[1] + cut[3]) / 2.0],
            angle: 0.0,
        };
    }
}

fn region_problem(
    problem: &pcb_placer::Problem,
    poses: &[pcb_placer::Pose],
    active: &BTreeSet<usize>,
    region: [f64; 4],
) -> pcb_placer::Problem {
    let mut sub = problem.clone();
    sub.poses = poses.to_vec();
    for (index, component) in sub.components.iter_mut().enumerate() {
        if active.contains(&index) {
            continue;
        }
        if !component.fixed {
            make_inert(component);
            continue;
        }
        if !component.pins.is_empty() {
            let b = body_box(component, poses[index]);
            if b[0] > region[2] || b[2] < region[0] || b[1] > region[3] || b[3] < region[1] {
                component.pins.clear();
            }
        }
    }
    cut_keepouts(&mut sub, region);
    let pulling: Vec<bool> = (0..sub.components.len())
        .map(|part| active.contains(&part) || !sub.components[part].pins.is_empty())
        .collect();
    sub.outline = clip(&problem.outline, region);
    sub.pieces.clear();
    let constraints = &mut sub.constraints;
    constraints
        .regions
        .retain(|(part, _)| active.contains(part));
    constraints.starts.retain(|(part, _)| active.contains(part));
    constraints
        .edges
        .retain(|(part, _, _)| active.contains(part));
    constraints
        .overhangs
        .retain(|(part, _, _)| active.contains(part));
    constraints.kept.retain(|kept| active.contains(&kept.part));
    constraints
        .followers
        .retain(|follower| active.contains(&follower.leader));
    let user = constraints.relations.len() - constraints.automatic;
    let mut relations = Vec::new();
    let mut automatic = 0;
    for (index, relation) in constraints.relations.drain(..).enumerate() {
        let relation = match relation {
            pcb_placer::constraints::Relation::NearAny { part, anchors, max } => {
                let anchors: Vec<_> = anchors
                    .into_iter()
                    .filter(|anchor| pulling[anchor.part()])
                    .collect();
                if anchors.is_empty() || !active.contains(&part) {
                    continue;
                }
                pcb_placer::constraints::Relation::NearAny { part, anchors, max }
            }
            other => {
                let parts = other.parts();
                if !parts.iter().flatten().all(|part| pulling[*part])
                    || !parts.iter().flatten().any(|part| active.contains(part))
                {
                    continue;
                }
                other
            }
        };
        if index >= user {
            automatic += 1;
        }
        relations.push(relation);
    }
    constraints.relations = relations;
    constraints.automatic = automatic;
    constraints.linked.clear();
    constraints.linked_partners.clear();
    constraints.link_pairs();
    // The measured escape correction routes the whole board.
    if let Some(escape) = &sub.escape
        && escape.measured.is_some()
    {
        let mut estimate = (**escape).clone();
        estimate.measured = None;
        sub.escape = Some(std::sync::Arc::new(estimate));
    }
    sub
}

/// One seed's placement of a panel whose parts the placement holds in
/// their sub-boards (`PlacerApplied::held`): each sub-board with the
/// ordinary placer in turn, alone in its region (the outline cut to it,
/// so that its own fullness decides its levels: the 20 x 20 4in1-mini is
/// 107 % full at the full spacing and places at the tight bodies, which
/// the whole panel's levels, its area not overfull, reached too late),
/// with a share of the seed's budget by its movable parts; then the
/// movable parts without a sub-board with the whole board, the placed
/// ones fixed (the budget left, a tenth at least). The placement reports
/// the loosest level of the sub-boards', field by field.
pub(crate) fn place_sub_boards(
    problem: &pcb_placer::Problem,
    sub: &PlacerSubBoards,
    config: &pcb_placer::Config,
) -> pcb_placer::Placement {
    let started = pcb_placer::work();
    let debug = std::env::var_os("PCB_PLACER_DEBUG").is_some();
    let mut boards: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for (part, piece) in sub.part_piece.iter().enumerate() {
        let component = &problem.components[part];
        if let Some(piece) = piece
            && !component.fixed
        {
            boards.entry(*piece).or_default().insert(part);
        }
    }
    let total: usize = boards.values().map(BTreeSet::len).sum::<usize>().max(1);
    let mut poses = problem.poses.clone();
    let mut relaxation: Option<pcb_placer::Relaxation> = None;
    let mut unplaced: Vec<usize> = Vec::new();
    let mut overfull = false;
    let mut frames = Vec::new();
    let mut global_iterations = 0;
    let mut global_overflow: f64 = 0.0;
    let count = boards.len();
    for (index, (piece, active)) in boards.iter().enumerate() {
        let region_problem = region_problem(problem, &poses, active, sub.pieces.boxes[*piece]);
        let mut board_config = config.clone();
        board_config.work_limit = config.work_limit.map(|limit| {
            if index + 1 == count {
                limit
                    .saturating_sub(pcb_placer::work() - started)
                    .max(limit / 10)
            } else {
                (limit as f64 * active.len() as f64 / total as f64) as u64
            }
        });
        let placed = pcb_placer::place(&region_problem, &board_config);
        if debug {
            eprintln!(
                "sub-boards: board {piece} placed: {} movable, {} unplaced, {:.1} s of work",
                active.len(),
                placed.unplaced.len(),
                placed.work as f64 / pcb_placer::work_per_second()
            );
        }
        for part in active {
            poses[*part] = placed.poses[*part];
        }
        unplaced.extend(placed.unplaced.iter().copied());
        overfull |= placed.overfull;
        global_iterations += placed.global_iterations;
        global_overflow = global_overflow.max(placed.global_overflow);
        frames.extend(placed.frames.iter().cloned());
        relaxation = Some(match relaxation {
            Some(before) => loosest(&before, &placed.relaxation),
            None => placed.relaxation.clone(),
        });
    }
    let held = poses.clone();
    let (_, unassigned) = sub.unassigned(problem);
    let relaxation = relaxation.unwrap_or_else(|| pcb_placer::place(problem, config).relaxation);
    let (poses, relaxation, overfull) = if unassigned.is_empty() {
        (poses, relaxation, overfull)
    } else {
        // The parts without a sub-board, with the whole board.
        let mut rest = problem.clone();
        rest.poses = poses.clone();
        for active in boards.values() {
            for part in active {
                rest.components[*part].fixed = true;
            }
        }
        let mut rest_config = config.clone();
        rest_config.work_limit = config.work_limit.map(|limit| {
            limit
                .saturating_sub(pcb_placer::work() - started)
                .max(limit / 10)
        });
        let placed = pcb_placer::place(&rest, &rest_config);
        unplaced.extend(placed.unplaced.iter().copied());
        let relaxation = loosest(&relaxation, &placed.relaxation);
        (placed.poses, relaxation, overfull || placed.overfull)
    };
    let mut poses = poses;
    problem.sync_followers(&mut poses);
    unplaced.sort_unstable();
    unplaced.dedup();
    let mut fin = problem.clone();
    relaxation.apply(&mut fin);
    let illegal = pcb_placer::legal::illegal_components(&fin, &poses);
    let constraints = pcb_placer::constraints::report(&fin, &poses);
    let wirelength_final = problem.wirelength(&poses);
    frames.push(pcb_placer::global::Frame {
        iteration: global_iterations + 1,
        overflow: global_overflow,
        wirelength: wirelength_final,
        poses: poses.clone(),
    });
    pcb_placer::Placement {
        frames,
        global_iterations,
        global_overflow,
        wirelength_initial: problem.wirelength(&problem.poses),
        wirelength_global: problem.wirelength(&held),
        wirelength_legal: problem.wirelength(&held),
        wirelength_final,
        unplaced,
        illegal,
        constraints,
        relaxation,
        work: pcb_placer::work() - started,
        stage_poses: vec![("sub-boards", held), ("final", poses.clone())],
        global_histories: Vec::new(),
        overfull,
        level: 0,
        skeleton: None,
        walk: Vec::new(),
        poses,
        exits: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pcb_router as core;

    /// Two 20 x 20 boards one above the other, joined by a 4 mm tab (with
    /// a row of mouse bites when `bites`); two parts on each, on GND and
    /// SIG; `extra` more items.
    fn panel(bites: bool, extra: &str) -> Expr {
        panel_with(bites, extra, "SIG")
    }

    /// `panel` with the second net named `signal`.
    fn panel_with(bites: bool, extra: &str, signal: &str) -> Expr {
        let outline = "(gr_poly (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy 12 20) (xy 12 25) (xy 20 25) (xy 20 45) (xy 0 45) (xy 0 25) (xy 8 25) (xy 8 20) (xy 0 20)) (layer \"Edge.Cuts\"))";
        let holes: String = if bites {
            (0..5)
                .map(|index| {
                    let x = 8.4 + 0.8 * index as f64;
                    format!(
                        "(gr_circle (center {x} 22.5) (end {} 22.5) (layer \"Edge.Cuts\"))",
                        x + 0.25
                    )
                })
                .collect()
        } else {
            String::new()
        };
        let part = |name: &str, x: f64, y: f64| {
            format!(
                r#"(footprint "{name}" (at {x} {y}) (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "{signal}")))"#
            )
        };
        let parts = [
            part("R1", 3.0, 5.0),
            part("R2", 13.0, 5.0),
            part("R1", 3.0, 35.0),
            part("R2", 13.0, 35.0),
        ]
        .join(" ");
        crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)) {outline} {holes} {parts} {extra})"#
        ))
        .unwrap()
    }

    /// A 20 x 20 board above a 20 x `lower` piece, joined by a bridge
    /// `bridge` mm wide at x 10 (y 20 to 25); `extra` more items.
    fn two_pieces(bridge: f64, lower: f64, extra: &str) -> Expr {
        let (left, right, bottom) = (10.0 - bridge / 2.0, 10.0 + bridge / 2.0, 25.0 + lower);
        let outline = format!(
            "(gr_poly (pts (xy 0 0) (xy 20 0) (xy 20 20) (xy {right} 20) (xy {right} 25) (xy 20 25) (xy 20 {bottom}) (xy 0 {bottom}) (xy 0 25) (xy {left} 25) (xy {left} 20) (xy 0 20)) (layer \"Edge.Cuts\"))"
        );
        crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user) (41 "Cmts.User" user)) {outline} {extra})"#
        ))
        .unwrap()
    }

    /// A footprint of `count` non-plated holes of `drill` mm at `pitch`
    /// along x from `x`, at y 22.5 (a row of mouse bites across the
    /// bridge).
    fn bites(x: f64, count: usize, pitch: f64, drill: f64) -> String {
        let pads: String = (0..count)
            .map(|index| {
                format!(
                    r#"(pad "" np_thru_hole circle (at {} 0) (size {drill} {drill}) (drill {drill}) (layers "*.Cu" "*.Mask"))"#,
                    index as f64 * pitch
                )
            })
            .collect();
        format!(
            r#"(footprint "Panelization:mouse-bites" (at {x} 22.5) (property "Reference" "MB") {pads})"#
        )
    }

    #[test]
    fn sub_boards_are_reported_from_the_outline_alone_with_no_part_on_any_piece() {
        // The panel's two boards with their four parts stacked off the
        // board (a netlist import), no reference: nothing tells which part
        // goes where, and the report says the pieces anyway.
        let board = |places: [[f64; 2]; 4]| {
            let parts: String = ["R1", "R2", "R3", "R4"]
                .iter()
                .zip(places)
                .map(|(name, at)| {
                    format!(
                        r#"(footprint "R" (at {} {}) (uuid "uuid-{name}") (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SIG-{name}")))"#,
                        at[0], at[1]
                    )
                })
                .collect();
            two_pieces(1.5, 20.0, &parts)
        };
        let pcb = board([[-30.0, -30.0]; 4]);
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        // Off the board they are held by default; `move_all` frees them.
        for component in &mut lowered.problem.components[..4] {
            component.fixed = false;
        }
        let sub = detect_for_placer(&pcb, &lowered.problem, 4, None)
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 0);
        assert!(!sub.has_frames());
        let report = KiCadSubBoardsReport::of_outline(&sub, &lowered.problem, &lowered.references);
        assert_eq!(
            (
                report.count,
                report.pieces,
                report.parts_assigned,
                report.unassigned
            ),
            (0, 2, 0, 4)
        );
        assert!(!report.cut_by.is_empty());
        assert!(!report.tabs.is_empty());
        assert_eq!(report.sub_boards.len(), 2);
        assert!(report.sub_boards.iter().all(|board| board.parts == 0));
        assert!(!report.held && report.kept_clear_mm2 == 0.0);
        assert!(report.boards_unknown());
        // describe-kicad-board says the same of the board.
        let described = describe_pieces(&pcb).unwrap().expect("pieces");
        assert_eq!(
            (
                described.pieces,
                described.parts_assigned,
                described.frames_by
            ),
            (2, 0, "shape")
        );
        assert_eq!(described.sub_boards.len(), 2);
        // With a reference (the parts at their places): as before, two
        // sub-boards from it, every part on one, no lead action.
        let reference = board([[3.0, 5.0], [13.0, 5.0], [3.0, 35.0], [13.0, 35.0]]);
        let sub = detect_for_placer(&pcb, &lowered.problem, 4, Some(&reference))
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 2);
        let mut placed = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        for component in &mut placed.problem.components[..4] {
            component.fixed = false;
        }
        let applied = apply_to_placer(
            &sub,
            &mut placed.problem,
            &mut placed.references,
            &mut placed.source_at,
            &mut placed.poured,
        );
        let report = KiCadSubBoardsReport::of(
            &sub,
            &placed.problem,
            &placed.references,
            &lowered.references,
            &applied,
        );
        assert_eq!(
            (report.count, report.unassigned, report.parts_assigned),
            (2, 0, 4)
        );
        assert_eq!(report.from, vec!["reference board"]);
        assert!(!report.boards_unknown());
    }

    #[test]
    fn tabs_are_narrow_bridges_rows_of_mouse_bites_and_v_cuts() {
        // A bridge of 1.5 mm: the outline's own neck.
        let narrow = Pieces::of_board(&two_pieces(1.5, 20.0, ""))
            .unwrap()
            .expect("pieces");
        assert_eq!(narrow.count(), 2);
        assert_eq!(narrow.cut_by, vec!["narrow outline"]);
        // A solid bridge of 6 mm is board.
        assert!(
            Pieces::of_board(&two_pieces(6.0, 20.0, ""))
                .unwrap()
                .is_none()
        );
        // Crossed by a row of 0.5 mm holes at 0.75 mm (a footprint of
        // non-plated pads, as the ULK's), from edge to edge.
        let row = bites(7.25, 8, 0.75, 0.5);
        let pcb = two_pieces(6.0, 20.0, &row);
        let cuts = Cuts::of_board(&pcb);
        assert_eq!((cuts.holes.len(), cuts.rows), (8, 1));
        let bitten = Pieces::of_board(&pcb).unwrap().expect("pieces");
        assert_eq!(bitten.count(), 2);
        assert_eq!(bitten.cut_by, vec!["mouse bites"]);
        assert!(bitten.between([10.25 + 0.375, 22.5]));
        assert_ne!(bitten.piece_at([10.0, 10.0]), bitten.piece_at([10.0, 35.0]));
        // Holes 1.5 mm apart leave bridges for tracks (protoesp's signal
        // tabs): no row, one board.
        let sparse = two_pieces(6.0, 20.0, &bites(7.25, 4, 1.5, 0.5));
        assert_eq!(Cuts::of_board(&sparse).rows, 0);
        assert!(Pieces::of_board(&sparse).unwrap().is_none());
        // Two holes are no row.
        assert_eq!(
            Cuts::of_board(&two_pieces(6.0, 20.0, &bites(9.0, 2, 0.75, 0.5))).rows,
            0
        );
        // A V-cut: a line on a user layer across the bridge, labelled at
        // its start (as KiKit writes it).
        let line = r#"(gr_line (start -1 22.5) (end 21 22.5) (stroke (width 0.4) (type default)) (layer "Cmts.User"))"#;
        let label = r#"(gr_text "V-CUT" (at -4 22.5 0) (layer "Cmts.User"))"#;
        let scored = two_pieces(6.0, 20.0, &format!("{line} {label}"));
        assert_eq!(Cuts::of_board(&scored).lines.len(), 1);
        let pieces = Pieces::of_board(&scored).unwrap().expect("pieces");
        assert_eq!(pieces.count(), 2);
        assert_eq!(pieces.cut_by, vec!["V-cuts"]);
        assert!(pieces.between([10.0, 22.5]));
        // Without its label the line is a drawing.
        assert!(
            Pieces::of_board(&two_pieces(6.0, 20.0, line))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_grid_of_v_cuts_is_reported_strip_by_strip() {
        // Trial 5's SmartSpin2k: nine copies on one outline cut by a grid
        // of V-cuts, reported as one tab over the whole panel. Here four
        // boards of 20 x 20 mm on one 41 x 41 mm outline, a V-cut across
        // each way.
        let outline =
            r#"(gr_poly (pts (xy 0 0) (xy 41 0) (xy 41 41) (xy 0 41)) (layer "Edge.Cuts"))"#;
        // Each labelled 3 mm before its start, on its axis.
        let cut = |start: [f64; 2], end: [f64; 2], label: [f64; 2]| {
            format!(
                r#"(gr_line (start {} {}) (end {} {}) (stroke (width 0.4) (type default)) (layer "Cmts.User")) (gr_text "V-CUT" (at {} {} 0) (layer "Cmts.User"))"#,
                start[0], start[1], end[0], end[1], label[0], label[1]
            )
        };
        let pcb = crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)) {outline} {} {})"#,
            cut([-1.0, 20.5], [42.0, 20.5], [-4.0, 20.5]),
            cut([20.5, -1.0], [20.5, 42.0], [20.5, -4.0])
        ))
        .unwrap();
        let pieces = Pieces::of_board(&pcb).unwrap().expect("pieces");
        assert_eq!(pieces.count(), 4, "{:?}", pieces.boxes);
        assert_eq!(pieces.cut_by, vec!["V-cuts"]);
        // Two strips, each along the panel and a cell or two across.
        assert_eq!(pieces.tabs.len(), 2, "{:?}", pieces.tabs);
        for tab in &pieces.tabs {
            let (width, height) = (tab[2] - tab[0], tab[3] - tab[1]);
            assert!(
                width.max(height) > 35.0 && width.min(height) <= 2.0 + 1.0e-9,
                "{tab:?}"
            );
        }
        // The description says no `movable` per board: it knows no
        // constraints.
        let described = describe_pieces(&pcb).unwrap().expect("pieces");
        let text = serde_json::to_string(&described).unwrap();
        assert!(!text.contains("movable"), "{text}");
        assert_eq!(described.tabs.len(), 2);
    }

    #[test]
    fn a_piece_without_parts_is_a_frame() {
        // The board above, a 5 mm rail below, joined by mouse bites.
        let pcb = two_pieces(6.0, 5.0, &bites(7.25, 8, 0.75, 0.5));
        let pieces = Pieces::of_board(&pcb).unwrap().expect("pieces");
        let board = pieces.piece_at([10.0, 10.0]).unwrap();
        let rail = pieces.piece_at([10.0, 28.0]).unwrap();
        assert_eq!(
            pieces.frames(&BTreeSet::from([board])),
            (0..2).map(|piece| piece == rail).collect::<Vec<_>>()
        );
        // Nothing known: no frame.
        assert_eq!(pieces.frames(&BTreeSet::new()), vec![false, false]);
        // A piece larger than half of those with parts is a board whose
        // parts are all movable, not a frame.
        let pcb = two_pieces(6.0, 15.0, &bites(7.25, 8, 0.75, 0.5));
        let pieces = Pieces::of_board(&pcb).unwrap().expect("pieces");
        let board = pieces.piece_at([10.0, 10.0]).unwrap();
        assert_eq!(pieces.frames(&BTreeSet::from([board])), vec![false, false]);
    }

    #[test]
    fn the_placeable_area_leaves_out_frames_and_tabs() {
        // The board above, a rail below; two placed parts on the board.
        let part = |name: &str, x: f64, y: f64| {
            format!(
                r#"(footprint "{name}" (at {x} {y}) (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SIG")))"#
            )
        };
        let extra = format!(
            "{} {} {}",
            bites(7.25, 8, 0.75, 0.5),
            part("R1", 4.0, 5.0),
            part("R2", 12.0, 15.0)
        );
        let pcb = two_pieces(6.0, 5.0, &extra);
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        let footprints = 3;
        let sub = detect_for_placer(&pcb, &lowered.problem, footprints, None)
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 1);
        assert!(sub.has_frames());
        let components = lowered.problem.components.len();
        let applied = apply_to_placer(
            &sub,
            &mut lowered.problem,
            &mut lowered.references,
            &mut lowered.source_at,
            &mut lowered.poured,
        );
        assert_eq!(applied.held, 0, "one sub-board: nothing to hold");
        assert!(applied.tabs > 0);
        let keepouts: Vec<[f64; 4]> = (components..lowered.problem.components.len())
            .map(|index| {
                body_box(
                    &lowered.problem.components[index],
                    lowered.problem.poses[index],
                )
            })
            .collect();
        let kept = |point: [f64; 2]| {
            keepouts.iter().any(|b| {
                point[0] >= b[0] && point[0] <= b[2] && point[1] >= b[1] && point[1] <= b[3]
            })
        };
        // The rail, the tab and a margin beside the tab are kept out; the
        // board is not.
        assert!(kept([10.0, 28.0]) && kept([2.0, 28.0]));
        assert!(kept([10.0, 22.5]));
        assert!(kept([10.0, 21.0]), "the margin above the tab: {keepouts:?}");
        assert!(!kept([10.0, 10.0]) && !kept([2.0, 18.0]));
        let mut placer = pcb_placer::Config::new();
        placer.global.seed = 1;
        placer.work_limit = Some((2.0 * pcb_placer::work_per_second()) as u64);
        let placement = pcb_placer::place(&lowered.problem, &placer);
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
        for name in ["R1", "R2"] {
            let part = lowered.references.iter().position(|r| r == name).unwrap();
            assert!(!lowered.problem.components[part].fixed);
            let b = body_box(&lowered.problem.components[part], placement.poses[part]);
            assert!(b[3] < 20.0, "{name} off the board: {b:?}");
        }
    }

    #[test]
    fn connectors_and_the_designers_overhangs_may_reach_over_frames_and_tabs() {
        // The board above, a rail below, joined over x 7 to 13 and cut
        // along y 22.5 by a V-cut; a resistor, a USB receptacle (its pads
        // at the back, its shell reaching 3.7 mm past them towards +y) and
        // a second resistor, placed on the board.
        let resistor = |name: &str, x: f64, y: f64| {
            format!(
                r#"(footprint "Resistor_SMD:R_0603" (at {x} {y}) (uuid "uuid-{name}") (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SIG")))"#
            )
        };
        let usb = |x: f64, y: f64| {
            format!(
                r#"(footprint "Connector_USB:USB_C_Receptacle" (at {x} {y}) (uuid "uuid-J1") (property "Reference" "J1") (fp_rect (start -2 -2) (end 2 3) (layer "F.CrtYd")) (pad "1" smd rect (at -1 -1) (size 0.6 0.6) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 1 -1) (size 0.6 0.6) (layers "F.Cu") (net "SIG")))"#
            )
        };
        let v_cut = r#"(gr_line (start -1 22.5) (end 21 22.5) (stroke (width 0.4) (type default)) (layer "Cmts.User")) (gr_text "V-CUT" (at -4 22.5 0) (layer "Cmts.User"))"#;
        let board = |r1: [f64; 2]| {
            two_pieces(
                6.0,
                5.0,
                &format!(
                    "{v_cut} {} {} {}",
                    resistor("R1", r1[0], r1[1]),
                    usb(4.0, 8.0),
                    resistor("R2", 12.0, 5.0)
                ),
            )
        };
        let pcb = board([3.0, 15.0]);
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let lowered_of = |reference: Option<&Expr>| {
            let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
            let footprints = 3;
            let sub = detect_for_placer(&pcb, &lowered.problem, footprints, reference)
                .unwrap()
                .expect("pieces");
            assert!(sub.has_frames());
            let applied = apply_to_placer(
                &sub,
                &mut lowered.problem,
                &mut lowered.references,
                &mut lowered.source_at,
                &mut lowered.poured,
            );
            (lowered, applied)
        };
        let part = |lowered: &crate::board_placer::LoweredPlacement, name: &str| {
            lowered.references.iter().position(|r| r == name).unwrap()
        };
        let legal_in = |problem: &pcb_placer::Problem, index: usize, at: [f64; 2]| {
            let pose = pcb_placer::Pose {
                position: at,
                angle: 0.0,
            };
            pcb_placer::legal::is_legal(
                problem,
                &problem.poses,
                index,
                pose,
                (0..problem.components.len()).filter(|other| problem.components[*other].fixed),
            )
        };
        let legal = |lowered: &crate::board_placer::LoweredPlacement, name: &str, at: [f64; 2]| {
            legal_in(&lowered.problem, part(lowered, name), at)
        };
        let at_origin = |lowered: &crate::board_placer::LoweredPlacement, index: usize| {
            body_box(
                &lowered.problem.components[index],
                pcb_placer::Pose {
                    position: [0.0, 0.0],
                    angle: 0.0,
                },
            )
        };
        let (lowered, applied) = lowered_of(None);
        assert_eq!((applied.connectors, applied.designer_overhangs), (1, 0));
        let frames = &lowered.problem.constraints.frames;
        assert_eq!(frames.passes.len(), 1);
        assert_eq!(frames.passes[0].0, part(&lowered, "J1"));
        assert!(frames.passes[0].1.connector);
        // The keepout over the cut reaches 1.5 mm up into the board, the
        // cut itself (`copper`) starts at about y 22.5.
        let top = frames
            .keepouts
            .iter()
            .map(|k| body_box(&lowered.problem.components[*k], lowered.problem.poses[*k]))
            .filter(|b| b[0] < 10.0 && b[2] > 10.0)
            .map(|b| b[1])
            .fold(f64::INFINITY, f64::min);
        let cut = frames
            .copper
            .iter()
            .filter(|b| b[0] < 10.0 && b[2] > 10.0)
            .map(|b| b[1])
            .fold(f64::INFINITY, f64::min);
        assert!(
            (cut - 22.4).abs() < 0.4 && (cut - top - 1.6).abs() < 0.4,
            "{top} {cut}"
        );
        // A resistor above the cut: off the keepout (by its halo and the
        // spacing) legal, on it not.
        let r1 = part(&lowered, "R1");
        let r1_box = at_origin(&lowered, r1);
        let above = |depth: f64| [9.0 - r1_box[0], top - r1_box[3] + depth];
        assert!(legal(&lowered, "R1", above(-1.5)));
        assert!(!legal(&lowered, "R1", above(0.5)));
        // The connector's shell may reach over the cut and the rail where
        // its pads (0.7 to 1.3 mm behind its origin) keep the edge margin
        // off the cut; not where they come closer.
        let j1 = part(&lowered, "J1");
        let j1_box = at_origin(&lowered, j1);
        assert!(j1_box[3] > 2.9, "{j1_box:?}");
        let margin = lowered.problem.edge_margin;
        let pads_at = |gap: f64| [10.0, cut - margin - gap + 0.7];
        assert!(legal(&lowered, "J1", pads_at(0.05)));
        assert!(!legal(&lowered, "J1", pads_at(-0.05)));
        // Without the pass its body keeps off the keepout.
        let mut strict = lowered.problem.clone();
        strict.constraints.frames.passes.clear();
        assert!(!legal_in(&strict, j1, pads_at(0.05)));
        assert!(legal_in(&strict, j1, [10.0, 8.0]));
        // The designer put R1 0.5 mm into the keepout: it may reach as far,
        // not farther.
        let designer = board(above(0.5));
        let (lowered, applied) = lowered_of(Some(&designer));
        assert_eq!((applied.connectors, applied.designer_overhangs), (1, 1));
        let pass = lowered
            .problem
            .constraints
            .frames
            .passes
            .iter()
            .find(|(part, _)| *part == r1)
            .expect("R1's pass")
            .1;
        assert!(
            !pass.connector && (pass.overhang - 0.5).abs() < 0.01,
            "{pass:?}"
        );
        assert!(legal(&lowered, "R1", above(0.5)));
        assert!(!legal(&lowered, "R1", above(0.6)));
        // R2 has no pass: it keeps off.
        let r2 = part(&lowered, "R2");
        let r2_box = at_origin(&lowered, r2);
        assert!(!legal(
            &lowered,
            "R2",
            [13.0 - r2_box[0], top - r2_box[3] + 0.5]
        ));
    }

    #[test]
    fn copies_keep_off_every_copys_tabs_and_frames() {
        // Two copies side by side (x 0 to 20 and 21.5 to 41.5), a rail at
        // the left (x -9.5 to -1.5); bridges over the 1.5 mm slots at y 8
        // to 12, cut by rows of mouse bites, between the rail and the
        // first copy and between the copies.
        let outline = "(gr_poly (pts (xy -9.5 0) (xy -1.5 0) (xy -1.5 8) (xy 0 8) (xy 0 0) (xy 20 0) (xy 20 8) (xy 21.5 8) (xy 21.5 0) (xy 41.5 0) (xy 41.5 20) (xy 21.5 20) (xy 21.5 12) (xy 20 12) (xy 20 20) (xy 0 20) (xy 0 12) (xy -1.5 12) (xy -1.5 20) (xy -9.5 20)) (layer \"Edge.Cuts\"))";
        let row = |x: f64| {
            let pads: String = (0..6)
                .map(|index| {
                    format!(
                        r#"(pad "" np_thru_hole circle (at 0 {}) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))"#,
                        index as f64 * 0.75
                    )
                })
                .collect();
            format!(
                r#"(footprint "Panelization:mouse-bites" (at {x} 8.125) (property "Reference" "MB") {pads})"#
            )
        };
        let pcb = crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)) {outline} {} {})"#,
            row(-0.75),
            row(20.75)
        ))
        .unwrap();
        let pieces = Pieces::of_board(&pcb).unwrap().expect("pieces");
        assert_eq!(pieces.count(), 3);
        let (first, second) = (
            pieces.piece_at([10.0, 10.0]).unwrap(),
            pieces.piece_at([31.5, 10.0]).unwrap(),
        );
        let frames = pieces.frames(&BTreeSet::from([first, second]));
        assert_eq!(frames.iter().filter(|frame| **frame).count(), 1);
        let copies = CopyFrames {
            transforms: vec![(0.0, [0.0, 0.0]), (0.0, [21.5, 0.0])],
            within: vec![[-1.5, -1.5, 20.0, 21.5], [20.0, -1.5, 41.5, 21.5]],
            held: vec![Vec::new(), Vec::new()],
        };
        let (keepouts, copper) = pieces.copies_keepouts(&frames, &copies, TAB_MARGIN_MM);
        let kept = |boxes: &[[f64; 4]], point: [f64; 2]| {
            boxes.iter().any(|b| {
                point[0] >= b[0] && point[0] <= b[2] && point[1] >= b[1] && point[1] <= b[3]
            })
        };
        // The rail and the tabs' margins as before.
        let own = pieces.keepouts(&frames, TAB_MARGIN_MM);
        assert!(kept(&keepouts, [-5.0, 10.0]) && kept(&own, [-5.0, 10.0]));
        assert!(kept(&keepouts, [19.6, 10.0]) && kept(&own, [19.6, 10.0]));
        // The tab at the first copy's right side, carried to the second
        // copy's right side, where it has none.
        assert!(!kept(&own, [41.1, 10.0]));
        assert!(kept(&keepouts, [41.1, 10.0]));
        // Each copy keeps off what the other does, in its own frame.
        for x in 0..40 {
            for y in 0..40 {
                let point = [0.3 + 0.5 * x as f64, 0.3 + 0.5 * y as f64];
                assert_eq!(
                    kept(&keepouts, point),
                    kept(&keepouts, [point[0] + 21.5, point[1]]),
                    "{point:?}"
                );
            }
        }
        // The copper: the tabs themselves, not their margins.
        assert!(kept(&copper, [20.75, 10.0]) && !kept(&copper, [18.5, 10.0]));
        // A part of the second copy held at the right edge: the first
        // copy's tab is not carried there; its own keepouts stay.
        let held = CopyFrames {
            held: vec![Vec::new(), vec![[38.0, -1.5, 41.5, 21.5]]],
            ..copies.clone()
        };
        let (keepouts, _) = pieces.copies_keepouts(&frames, &held, TAB_MARGIN_MM);
        assert!(!kept(&keepouts, [41.1, 10.0]));
        assert!(kept(&keepouts, [22.0, 10.0]) && kept(&keepouts, [19.6, 10.0]));
    }

    #[test]
    fn a_held_sub_boards_problem_cuts_the_keepouts_to_its_region() {
        // Two 20 x 20 halves (y 0 to 20 and 25 to 45) joined by mouse bites
        // over x 7 to 13, and a 9 x 45 mm rail at their left (x -12 to
        // -3) joined to each by mouse bites over 4 mm: the keepouts take
        // more than a half's area.
        let outline = "(gr_poly (pts (xy -12 0) (xy -3 0) (xy -3 8) (xy 0 8) (xy 0 0) (xy 20 0) (xy 20 20) (xy 13 20) (xy 13 25) (xy 20 25) (xy 20 45) (xy 0 45) (xy 0 37) (xy -3 37) (xy -3 45) (xy -12 45)) (layer \"Edge.Cuts\")) (gr_poly (pts (xy -3 12) (xy 0 12) (xy 0 20) (xy 7 20) (xy 7 25) (xy 0 25) (xy 0 33) (xy -3 33)) (layer \"Edge.Cuts\"))";
        let column = |y: f64| {
            let pads: String = (0..6)
                .map(|index| {
                    format!(
                        r#"(pad "" np_thru_hole circle (at 0 {}) (size 0.5 0.5) (drill 0.5) (layers "*.Cu" "*.Mask"))"#,
                        index as f64 * 0.75
                    )
                })
                .collect();
            format!(
                r#"(footprint "Panelization:mouse-bites" (at -1.5 {y}) (property "Reference" "MB") {pads})"#
            )
        };
        let part = |name: &str, x: f64, y: f64| {
            format!(
                r#"(footprint "Resistor_SMD:R_0603" (at {x} {y}) (property "Reference" "{name}") (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "GND")) (pad "2" smd rect (at 2 0) (size 1 1) (layers "F.Cu") (net "SIG")))"#
            )
        };
        let pcb = crate::kicad_sexpr::parse(&format!(
            r#"(kicad_pcb (layers (0 "F.Cu" signal) (2 "B.Cu" signal) (25 "Edge.Cuts" user)) {outline} {} {} {} {} {} {} {})"#,
            part("R1", 5.0, 5.0),
            part("R2", 12.0, 15.0),
            part("R3", 5.0, 30.0),
            part("R4", 12.0, 40.0),
            column(8.125),
            column(33.125),
            bites(7.25, 8, 0.75, 0.5)
        ))
        .unwrap();
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        let sub = detect_for_placer(&pcb, &lowered.problem, 4, None)
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 2);
        assert!(sub.has_frames());
        let applied = apply_to_placer(
            &sub,
            &mut lowered.problem,
            &mut lowered.references,
            &mut lowered.source_at,
            &mut lowered.poured,
        );
        assert_eq!(applied.held, 4);
        let problem = &lowered.problem;
        let first = sub.part_piece[lowered.references.iter().position(|r| r == "R1").unwrap()]
            .expect("R1's half");
        let region = sub.pieces.boxes[first];
        let active: BTreeSet<usize> = (0..4)
            .filter(|part| sub.part_piece[*part] == Some(first))
            .collect();
        let kept = |problem: &pcb_placer::Problem| -> f64 {
            problem
                .constraints
                .frames
                .keepouts
                .iter()
                .filter(|k| problem.components[**k].side != pcb_placer::Side::Neither)
                .map(|k| problem.components[*k].body_size[0] * problem.components[*k].body_size[1])
                .sum()
        };
        let half = region_problem(problem, &problem.poses, &active, region);
        let outline = &half.outline;
        let area = (0..outline.len())
            .map(|i| {
                let (a, b) = (outline[i], outline[(i + 1) % outline.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0;
        // The keepouts whole: more than the half's area.
        assert!(kept(problem) > area, "{} of {area}", kept(problem));
        // Cut to the half: its tabs' margins only.
        assert!(kept(&half) < 0.25 * area, "{} of {area}", kept(&half));
        // So the first level (full halos) places the half.
        let mut placer = pcb_placer::Config::new();
        placer.global.seed = 1;
        placer.work_limit = Some((2.0 * pcb_placer::work_per_second()) as u64);
        let placement = pcb_placer::place(&half, &placer);
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
        assert!(
            placement.relaxation.halo_scale > 0.0,
            "{:?}",
            placement.relaxation
        );
        // Whole, the keepouts left only the last level.
        let mut whole = half.clone();
        whole.components = problem.components.clone();
        whole.poses = problem.poses.clone();
        for part in 0..problem.components.len() {
            if !active.contains(&part) && !problem.components[part].fixed {
                whole.components[part] = half.components[part].clone();
            }
        }
        let placement = pcb_placer::place(&whole, &placer);
        assert_eq!(
            placement.relaxation.halo_scale, 0.0,
            "{:?}",
            placement.relaxation
        );
    }

    #[test]
    fn mouse_bites_cut_the_outline_into_pieces() {
        let pieces = Pieces::of_board(&panel(true, "")).unwrap().expect("pieces");
        assert_eq!(pieces.count(), 2);
        assert_eq!(pieces.tabs.len(), 1, "{:?} {:?}", pieces.tabs, pieces.boxes);
        let top = pieces.piece_at([10.0, 10.0]).unwrap();
        let bottom = pieces.piece_at([10.0, 35.0]).unwrap();
        assert_ne!(top, bottom);
        assert!(pieces.between([9.6, 22.5]));
        assert!(!pieces.between([30.0, 22.5]));
        // A solid 4 mm tab is part of the board: one piece.
        assert!(Pieces::of_board(&panel(false, "")).unwrap().is_none());
    }

    #[test]
    fn nets_of_one_name_on_two_pieces_are_split_unless_copper_joins_them() {
        let pcb = panel(true, "");
        let pieces = Pieces::of_board(&pcb).unwrap().unwrap();
        assert_eq!(
            board_split_nets(&pcb, &pieces),
            BTreeMap::from([("GND".to_string(), 2), ("SIG".to_string(), 2)])
        );
        let config = crate::board_router::KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..Default::default()
        };
        let board = crate::board_router::lower(&pcb, &config, true)
            .unwrap()
            .board;
        let names: Vec<&str> = board.nets.iter().map(|net| net.name.as_str()).collect();
        assert_eq!(names.iter().filter(|name| **name == "GND").count(), 2);
        assert_eq!(names.iter().filter(|name| **name == "SIG").count(), 2);
        for (index, net) in board.nets.iter().enumerate() {
            assert_eq!(net.terminals.len(), 2, "{}", net.name);
            let ends: BTreeSet<Option<usize>> = net
                .terminals
                .iter()
                .map(|terminal| pieces.piece_at(terminal.anchor))
                .collect();
            assert_eq!(ends.len(), 1, "{}", net.name);
            for terminal in &net.terminals {
                assert_eq!(
                    board.obstacles[terminal.pad].net,
                    Some(index as core::NetId)
                );
            }
        }
        // A track of SIG over the tab joins it: SIG stays one net.
        let joined = panel(
            true,
            r#"(segment (start 10 10) (end 10 40) (width 0.2) (layer "B.Cu") (net "SIG"))"#,
        );
        assert_eq!(
            board_split_nets(&joined, &pieces),
            BTreeMap::from([("GND".to_string(), 2)])
        );
        let board = crate::board_router::lower(&joined, &config, true)
            .unwrap()
            .board;
        assert_eq!(board.nets.iter().filter(|net| net.name == "SIG").count(), 1);
        // Switched off: one net each.
        let off = crate::board_router::KiCadBoardRouterConfig {
            sub_board_nets: Some(false),
            ..config.clone()
        };
        let board = crate::board_router::lower(&pcb, &off, true).unwrap().board;
        assert_eq!(board.nets.iter().filter(|net| net.name == "GND").count(), 1);
    }

    #[test]
    fn kicad_items_across_two_sub_boards_are_not_counted() {
        let pcb = panel(true, "");
        let pieces = Pieces::of_board(&pcb).unwrap().unwrap();
        let item = |net: &str, a: [f64; 2], b: [f64; 2]| {
            serde_json::json!({"type": "unconnected_items", "items": [
                {"description": format!("Pad 1 [{net}] of R1 on F.Cu"), "pos": {"x": a[0], "y": a[1]}},
                {"description": format!("Pad 1 [{net}] of R2 on F.Cu"), "pos": {"x": b[0], "y": b[1]}}]})
        };
        let mut report = serde_json::json!({"unconnected_items": [
            item("GND", [3.0, 5.0], [3.0, 35.0]),
            // A second GND edge across: an island within a piece left open.
            item("GND", [13.0, 5.0], [13.0, 35.0]),
            item("SIG", [5.0, 5.0], [15.0, 5.0]),
        ]});
        let mut only_sig = report.clone();
        assert_eq!(excuse_across(&mut report, &pcb, &pieces, None), 1);
        assert_eq!(report["unconnected_items"].as_array().unwrap().len(), 2);
        assert_eq!(report[ACROSS_KEY].as_array().unwrap().len(), 1);
        // A placement that knew GND not to be on both boards by design:
        // its items across count.
        let designed = BTreeSet::from(["SIG".to_string()]);
        assert_eq!(
            excuse_across(&mut only_sig, &pcb, &pieces, Some(&designed)),
            0
        );
    }

    #[test]
    fn a_placed_panel_holds_each_part_on_its_board() {
        // A hierarchical name: the placer's connection drops the slash,
        // KiCad's report keeps it.
        let pcb = panel_with(true, "", "/S/SIG");
        let config = crate::board_placer::KiCadBoardPlacerConfig::default();
        let mut lowered = crate::board_placer::lower_placement(&pcb, &config).unwrap();
        let sub = detect_for_placer(&pcb, &lowered.problem, 4, None)
            .unwrap()
            .expect("pieces");
        assert_eq!(sub.count(&lowered.problem), 2);
        assert_eq!(sub.from, vec!["placed input"]);
        assert_eq!(sub.split.len(), 2, "GND and SIG on both boards");
        assert_ne!(sub.part_piece[0], sub.part_piece[2], "the two R1 apart");
        let designed = designed_nets(&pcb, &sub, &lowered.net_names);
        assert_eq!(
            designed,
            BTreeSet::from(["/S/SIG".to_string(), "GND".to_string()])
        );
        let applied = apply_to_placer(
            &sub,
            &mut lowered.problem,
            &mut lowered.references,
            &mut lowered.source_at,
            &mut lowered.poured,
        );
        assert_eq!((applied.held, applied.nets_split), (4, 2));
        assert!(applied.tabs >= 1);
        // The two boards' R1 no longer share a net.
        let nets = |part: usize| -> Vec<usize> {
            lowered.problem.components[part]
                .pins
                .iter()
                .map(|pin| pin.net)
                .collect()
        };
        assert!(nets(0).iter().all(|net| !nets(2).contains(net)));
        let mut placer = pcb_placer::Config::new();
        placer.global.seed = 1;
        placer.work_limit = Some((2.0 * pcb_placer::work_per_second()) as u64);
        let placement = place_sub_boards(&lowered.problem, &sub, &placer);
        assert!(placement.unplaced.is_empty() && placement.illegal.is_empty());
        for part in 0..4 {
            let piece = sub.part_piece[part].unwrap();
            let at = placement.poses[part].position;
            assert_eq!(
                sub.pieces.piece_at(at),
                Some(piece),
                "part {part} at {at:?}"
            );
        }
    }

    #[test]
    fn a_panels_cells_split_only_the_nets_named_by_design() {
        // One outline piece (a solid tab): no pieces; the cells a
        // placement found split the nets it names, GND here, alone.
        let pcb = panel(false, "");
        let config = crate::board_router::KiCadBoardRouterConfig {
            default_rules: Some(crate::KiCadConnectionRoutingRules {
                trace_width_mm: 0.2,
                clearance_mm: 0.2,
                via_size_mm: 0.6,
                via_drill_mm: 0.3,
            }),
            ..Default::default()
        };
        let mut board = crate::board_router::lower(&pcb, &config, true)
            .unwrap()
            .board;
        assert_eq!(board.nets.len(), 2);
        let cells = Pieces::of_boxes(vec![[0.0, 0.0, 20.0, 22.0], [0.0, 23.0, 20.0, 45.0]]);
        assert_eq!(cells.piece_at([5.0, 30.0]), Some(1));
        let only = BTreeSet::from(["GND".to_string()]);
        let split = split_router_nets(&mut board, &cells, Some(&only));
        assert_eq!(split.nets, vec![("GND".to_string(), 2)]);
        assert_eq!(board.nets.iter().filter(|net| net.name == "GND").count(), 2);
        assert_eq!(board.nets.iter().filter(|net| net.name == "SIG").count(), 1);
    }
}

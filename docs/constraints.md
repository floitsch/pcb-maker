# Placement constraints (`constraints.json`)

An agent (or a person) says where parts belong; pcb-maker places the rest by
wirelength and routes the board. Constraints go into the placer config, as a
file next to the project or inline:

```sh
# layout.json: {"placer": {"constraints": "constraints.json"}}
pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto layout.json
pcb-maker place-kicad-board  <project-dir> <board-id> <out-dir> placer.json   # placement only
```

The file path is relative to the project directory. `{"placer": {"constraints": {...}}}`
takes the constraints inline.

## Format (version 1)

```json
{"version": 1,
 "outline":  {"width": 100, "height": 80, "x": 0, "y": 0},
 "move_all": true,
 "fixed":    ["J1", "H*"],
 "rotation": [{"part": "U1", "angle": 90}, {"part": "D1", "angles": [0, 180]}],
 "edge":     [{"part": "J4", "edge": "left", "flush": true},
              {"part": "U3", "edge": "left", "flush": true, "overhang": {"edge": "top"}}],
 "region":   [{"parts": ["U5", "C1?"], "x": [0, 20], "y": [0, 15]}],
 "keepout":  [{"x": [30, 40], "y": [0, 10], "side": "front", "copper": true}],
 "back":     ["BT1", "R1?"],
 "hollow":   ["SHIELD1"],
 "near":     [{"part": "C1", "pin_of": "U1:48", "max_mm": 3},
              {"part": "U2", "part_of": "J4", "max_mm": 10}],
 "relative": [{"part": "J4", "below": "U3", "max_gap_mm": 3}],
 "group":    [{"parts": ["U3", "L1", "C5?"], "max_mm": 4}],
 "row":      [{"parts": ["D1", "D2", "D3", "D4"], "pitch_mm": 5, "axis": "x"}],
 "apart":    [{"parts": ["U5"], "from": ["U1", "Q*"], "min_mm": 10}],
 "device_front": "left",
 "place":    [{"part": "SW1", "at": "front"}, {"part": "D1", "at": "top-right"},
              {"part": "J1", "x": 10, "y": 5, "angle": 90}],
 "add":      [{"part": "H1", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad", "nets": {"1": "GND"},
               "x": 3.5, "y": 3.5}]}
```

A `comment` (a text or a list of texts) is for the reader and is not
read: `import-kicad-netlist`'s `constraints.starter.json` says there what
it inferred ([agents.md](agents.md#0-from-a-schematic)).

Parts are named by reference. `fixed`, `back`, `front`, `hollow` and the
`parts` of `region`, `group` and `row` also take glob patterns (`*`, `?`).
A reference several footprints share (a panel's copies) names all of them
in `fixed` and the other lists; an `edge`, `rotation` or `place` entry for
it is about one footprint: the one whose `uuid` the entry gives
(`{"part": "J3", "edge": "left", "uuid": "0000...613f4ae5"}`; the
footprint's `uuid`, or `tstamp` in KiCad 5 to 7 files, as
`describe-kicad-board` lists it), or without one, in turn: the n-th entry
of that kind for the reference is about its n-th footprint in board
order. A `uuid` no footprint of that reference has is an error. Coordinates are millimetres, in KiCad's orientation: y grows
downwards, so `top` is the smaller y.

| Key | Meaning | Kind |
| --- | --- | --- |
| `outline` | The board becomes a `width` × `height` rectangle, replacing the board's Edge.Cuts. `x`/`y` place its top-left corner (default: the old outline's corner; without one, centred on the footprints). Parts lying entirely off the new board are placed, not kept. A `polygon`, round corners (`corner_radius`), holes (`cutouts`) and several `pieces`: see [Outlines that are not rectangles](#outlines-that-are-not-rectangles). Leave out `width` and `height` and the board is sized from its parts: `area_factor` times their total body area, at `aspect` width/height (default 1.5), with both sides kept within 100 mm when the area allows (the cheap fabs' flat price, see [cost.md](cost.md)). Without `area_factor`, `layout-kicad-board` starts where designers do (2 on two layers, 1.5 on more: the medians of 616 open-source boards are 1.9 and 1.4) and grows the board by a quarter, up to three times, while the parts do not fit or a quick route leaves connections open; with `"shrink": true` it instead shrinks a board that routes by a fifth while it still routes (the smallest board). The sizes tried are in `board-layout.json` as `outline_sizing`; the size used is reported as `outline_mm`. | board |
| `move_all` | Every footprint is placed, unless locked or named by `fixed`; mounting holes no constraint names go to the corners, then to the middle of the edges (reported as `edge` constraints). Use it for a board fresh from a netlist, or to lay out a placed board anew: where the parts start does not matter (lpc1311-target-v1 placed from the designer's positions and from one stack came out the same, part for part, 2026-10-26; the angles neither, [placer.md](placer.md#orientation-search-from-a-canonical-start-2026-10-15)), so there is no need to stack them first. Without it, default rules keep parts that look deliberately placed: connectors at the edge, parts over the outline. | board |
| `fixed` | These parts keep the pose they have in the board file. | hard |
| `place` | Where one part goes, in words or exactly. `at`: `left`, `right`, `top`, `bottom` (on that edge, as `edge` with the default 1 mm), `top-left`, `top-right`, `bottom-left`, `bottom-right` (in that corner), `center` (the middle third both ways), `left-half`, `right-half`, `top-half`, `bottom-half`, or `front`/`rear`. Or `x`, `y` (the footprint's origin; `origin` as for `region`) and optionally `angle`: the part is put there and kept (listed as kept, `place`, in board-layout.json's `constraints`). | hard |
| `add` | Parts the board needs and the schematic does not have: mounting holes plated on a net, a footprint for another switch, a test point. `import-kicad-netlist` puts them on the board (see [Parts the schematic does not have](#parts-the-schematic-does-not-have-add)); the layout keeps those with `x` and `y` there, as `place` does. | hard |
| `device_front` | The board edge the device's front is at (`left`, `right`, `top`, `bottom`), so that `place` can say `front` (a button the user presses) and `rear` (the power jack). | board |
| `rotation` | The part may only take these KiCad orientations (degrees). | hard |
| `edge` | The part's body lies within `max_mm` (default 1; 0 with `flush`) of that edge of the outline's bounding box (`left`, `right`, `top`, `bottom`, or `any`: the edge nearest to where the part settles in a placement without this constraint; `edges_chosen` in `board-placer.json` says which). On a `polygon` outline, of the nearest side of the polygon that faces that way (below). On that side the courtyard may touch the edge, but the part's pads keep the board's copper-to-edge clearance from it, and from every other side of the board (an overhang's side too): the placer holds them back by `copper_edge_clearance_mm` (the project's `min_copper_edge_clearance` unless the placer config sets it), the router keeps its tracks and vias as far from the edge, and KiCad's DRC reports a pad or track closer as `copper_edge_clearance`. This is the one description of copper at the edge; the other documents point here. | hard |
| `edge.opening_outwards` | The connector opens towards that edge: its mouth (the side where its courtyard reaches farthest beyond its pads, such as a USB receptacle's shell) faces out. Only orientations that do so are allowed; a footprint without a clear mouth is an error (`cannot tell which way it opens`, an overhanging USB-C receptacle's courtyard among them): drop `opening_outwards` and give the part a `rotation` that faces the edge, such as the angle `describe-kicad-board` lists for a board where it already faces it. | hard |
| `edge.overhang` | The part reaches beyond `overhang.edge` with its footprint's keepout zone (an ESP32 module's antenna). The keepout lies outside the board, flush with the edge; the rest of the body inside. Only orientations that point the keepout at that edge are allowed. The footprint must have a keepout zone. | hard |
| `region` | The bodies stay inside the box. By default `x`/`y` are relative to the top-left corner of the outline's bounding box; `"origin": "absolute"` uses board coordinates. In place of `x` and `y`, `at` names a part of the outline's box in words: `left-half`, `right-half`, `top-half`, `bottom-half` or `center` (the middle third both ways), whatever size the outline gets (a keyboard half on an outline sized from the parts). | hard |
| `keepout` | No part's body enters the box (`x`, `y` and `origin` as for `region`) on `side` `front`, `back` or `both` (default). With `"copper": true` no track, via or pour enters it either (under an antenna, for a label or a mechanical part). The box is written to the board as a KiCad rule area named `constraint keepout N`, so KiCad's DRC checks it too; placing again with the same constraints replaces it. A box that meets the courtyard of a part the placer keeps where it is (decided as the lowering decides it, `held_parts`: the placer's `fixed` and `fixed_patterns`, this file's `fixed` and parts it places with `x` and `y`, and by default locked parts, parts without a net and parts at the edge, unless `free`, a constraint naming them or `move_all` frees them) is written with footprints allowed: KiCad would report that part, put there on purpose; the placer keeps movable parts out by the area's name all the same, and with `"copper": true` tracks, vias and pours stay out for KiCad too (the held part's pads inside the box are then reached from outside it only). The input check warns of such a box before the run (the log and the verdict's next_actions), and of two held parts on top of each other (their courtyards on one side sharing half the smaller one, or their pads meeting on a copper layer), naming both. | hard |
| `back`, `front` | These parts go on the bottom (top) side. A part on the other side is flipped as KiCad flips it (mirrored, layers swapped) before placement; it is then placed like any other. A part named in both is an error. | hard |
| `hollow` | Only these parts' pads (and holes) block other parts; the rest of their courtyard may hold parts: a shield's outline around its headers, a module mounted above parts. Parts with holes (through-hole parts, mounting holes) still stay out of the courtyard, as KiCad's DRC forbids holes inside one; where the courtyard is drawn as several separate shapes (a Raspberry Pi outline marking only its connectors), only those shapes. KiCad's DRC will report the courtyard overlap it allows. Usually fixed as well. | hard |
| `near` | The gap between the part's body and another part's body (`part_of`) or a pad (`pin_of`, `REF:PAD`) is at most `max_mm`. | soft |
| `group` | The parts (references or globs) stay together: each body within `max_mm` of the central part's body, `around` (default: the largest member). Reported as one `near` per member. | soft |
| `row` | The parts (in the order given; a glob's parts in natural order, D2 before D10) lie in a line along the board's `axis` (`x`, default, or `y`), `pitch_mm` apart, turned alike. With `columns`, the parts fill a grid line by line (a keyboard's switches), the lines `row_pitch_mm` apart (default `pitch_mm`). The row is placed as one part: it moves and turns (by half turns) as a whole. Other constraints name its first part only, and then hold the whole row (`"below": "SW1"` is below the grid); naming another part of a row is an error. The exception: `near`'s `part_of` and `pin_of` may name any part of a row (a keyboard's diode at its switch), measured to that part's place in the row and reported against it. | hard |
| `apart` | Every body of `parts` stays at least `min_mm` from every body of `from` (references or globs): a temperature sensor away from the regulator, an audio input away from the switcher. Reported per pair. | soft |
| `relative` | The part lies `below`/`above`/`left_of`/`right_of` another part: it does not overlap it along that axis, its centre lies within the other part's extent across it, and the gap is at most `max_gap_mm` (default 5). | soft |

A part named by any constraint may move even where a default rule would fix
it (a connector at the edge, a part hanging over the outline), unless `fixed`
names it.

**Hard** constraints are part of legality. Every placement stage keeps them:
global placement, annealing, legalization, refinement and the router-driven
nudges. A board where no legal position exists reports the part as `unplaced`.

**Soft** constraints are a penalty: one millimetre of violation costs as
much as `constraint_weight` (default 50) millimetres of wire. They act as a
force in global placement. A repair pass then searches around every violated
relation, and router nudges may not make a violated one worse. When the
geometry makes a soft constraint impossible, the placement comes as close as
it can, and the report says by how much it misses.

## What comes back

`board-placer.json` (placement) and `board-layout.json` (place and route)
list every constraint:

```json
"constraints": [
  {"kind": "edge", "part": "U3", "satisfied": true, "violation_mm": 0.0},
  {"kind": "overhang", "part": "U3", "satisfied": true, "violation_mm": 0.0},
  {"kind": "near", "part": "C5", "other": "U3", "satisfied": false, "violation_mm": 3.235},
  {"kind": "rotation", "part": "J1", "satisfied": true, "violation_mm": 0.0},
  {"kind": "back", "part": "BT1", "satisfied": true, "violation_mm": 0.0}
]
```

`back`, `front`, `rotation` and an edge's `opening_outwards` are kept by
construction (the placer never changes a part's side or takes an angle
not allowed); they are listed so that every constraint has its line.

`constraint_warnings` lists accepted but unused inputs. An unknown key, part
or pad is an error, never ignored. So is a constraint no placement can keep,
said at once rather than after every relaxation level: a `region` smaller
than its part, a `row` longer than the outline in every way it may turn, or
parts held at one edge (each too deep to stand two behind each other) that
need more length than that edge has. With an outline sized from the parts
(`"outline": {}`) the outline checks are left out: the layout grows it.

## What pcb-maker adds by itself

Unless told otherwise (`"auto_decoupling": false` in the placer config),
the placer also pulls:
- every capacitor between a supply rail and ground to within 2.5 mm
  (body to pin, `decoupling_mm`) of a supply pin of an IC on that rail
  (5 mm for 4.7 µF and more): each IC's first supply pin on the rail,
  whichever IC suits. With `"decoupling_near": true` (placer config, off
  by default; [placer.md](placer.md#decoupling-at-the-pins-2026-10-26)):
  within 1 mm of whichever of the rail's supply pins on the ICs is
  nearest;
- every crystal to within 3 mm of the IC pins it drives;
- every inductor between a rail and an IC pin (a switcher's switch node)
  to within 3 mm of that pin;
- every ESD or surge protector (TVS, USBLC6, TPD4E001, ... by value) to
  within 3 mm of a connector pin it guards.

These are soft, like `near`, and are not reported among the constraints.
Parts a constraint already places keep to it; a `near` to a part's body
still leaves room for its pin. Parts tied together by `near` (yours or
these) may come closer than the routing halo of a big IC keeps others.
With `decoupling_near` on, a part near another (yours or these) is legalised
right after it, from where the anneal put it moved as far as that part
moved, and pays for the relation at each spot it is offered: a capacitor
the anneal put at its IC's pin stays at the pin when the IC moves.

To check them, or to set them per capacitor, write them down:
`describe-kicad-board <dir> <id> --constraints-template <file>` writes
each decoupling capacitor's `near` to the closest supply pin of its IC
(on a placed board; on a stacked one the rail's pins in turn) at 1.5 mm,
with the plug-in connectors as `edge` entries ([agents.md](agents.md#1-look-at-the-board)).
A layout under 60 % of its capacitors within 3 mm of a pin says so in its
verdict's next_actions.

One more rule is a placer config switch, on by default: on a board of
several pieces (a board with a breakaway beside it), the parts a net
joins to fixed parts on one piece stay on that piece (as a `region`);
`"keep_on_pieces": false` switches it off. (The placer option
`bus_rows`, rows of identical bus parts, was off, won nothing and was
removed on 2026-10-16; a placer config that sets it is rejected.)

The board's own texts and silkscreen graphics need no constraint: a
visible board text or text box on F.SilkS, B.SilkS, F.Cu or B.Cu (by
the box of its glyphs) and the board's silkscreen graphics (by their
strokes, in clusters; not a frame or an image spanning the board) are
obstacles on their side as a `keepout` for that side is (except one
lying on a movable part at its input position, which labels that part:
no obstacle, reported as `labels.attached`), and stay where the input
put them: the polish after routing no longer moves them, and
the router routes around copper texts. A fixed part already on one gets
a warning, not an error. A part that finds no room off them (a
connector at the edge under its own pin names) lies on them, and the
parts on them are reported in the verdict's next_actions. So a header's pin names or a test pad's name need no
keepout over them (the breadboard fence used three as a workaround);
`"keep_board_marks": false` in the placer config restores the behaviour
before 2026-10-21 (no obstacles, the polish moves them off the parts).
See [placer.md](placer.md), "The board's own texts and silkscreen
graphics".

A footprint's own rule areas (a U.FL's or an antenna's keepout) move and
turn with it. One that forbids pads but allows footprints keeps the other
side's pads out where the part is; a board rule area on inner layers only
that forbids pads keeps pads out on both sides (a through-hole pad
crosses the inner layers).

## Outlines that are not rectangles

`outline` also takes a polygon, round corners, holes and several pieces.
They are written to Edge.Cuts as lines and arcs (KiCad's DRC reads them
as a closed outline), the placer keeps parts on the shape itself (not its
box), and the router routes within it.

```json
"outline": {"polygon": [[0, 0], [115, 0], [115, 30], [230, 30], [230, 125], [0, 125]],
            "x": 0, "y": 0, "corner_radius": 2}
```

- **`polygon`**: the corners in millimetres, closed implicitly (a last
  point repeating the first is dropped). Another board's outline:
  `describe-kicad-board`'s `outline.points` of that board, as they are
  (with `x`, `y` its `outline.minimum` to keep its coordinates). The points are moved so that
  their bounding box's top-left corner lies at `x`, `y` (default as for
  the rectangle). Three corners at least, an area, no side crossing
  another: otherwise an input error (exit 2).
- **`corner_radius`**: every corner of the rectangle or polygon rounded
  with this radius, as an arc (inner corners too). A side too short for
  two full radii gives each of its corners half its length: a 30 x 4 mm
  outline with radius 5 becomes a slot with round ends.
- **`cutouts`**: holes, each a box (`"x": [10, 20], "y": [5, 10]`) or a
  `polygon`, optionally with its own `corner_radius`, relative to the
  outline's `x`, `y` (a piece's own corner; `"origin": "absolute"`:
  board coordinates).
  Each must lie inside its outline and apart from the others. No part's
  body enters a cutout and copper keeps the edge clearance from it (a
  cutout a footprint brings, a connector's slot, keeps only copper off).
  The room's usable area leaves them out.

  ```json
  "outline": {"width": 230, "height": 95, "corner_radius": 5,
              "cutouts": [{"x": [110, 120], "y": [20, 75]},
                          {"polygon": [[60, 80], [70, 80], [65, 88]]}]}
  ```
- **`pieces`**: several outlines in one board (a board with a breakaway
  part, two boards made as one): each piece a rectangle or polygon with its
  own `corner_radius` and `cutouts`, and its own `x`, `y`: where its
  top-left corner lies within the set (default 0, 0). The outline's `x`,
  `y` place the set's top-left corner. Pieces may not overlap or touch.
  No tabs or mouse bites are drawn. On a board of several pieces the
  parts a net joins to fixed parts on one piece stay on that piece
  (`keep_on_pieces` in the placer config, on by default), and a part held
  at an edge is held at its own piece's edge.

  ```json
  "outline": {"x": 0, "y": 0,
              "pieces": [{"width": 60, "height": 40, "corner_radius": 3},
                         {"polygon": [[0, 0], [30, 0], [30, 40], [0, 20]], "x": 65}]}
  ```

**Edges on a polygon.** Each side of the polygon faces left, right, top
or bottom: the way its outward normal points most (a side at exactly 45
degrees faces top or bottom; the segments of a rounded corner face
whichever way they point). An `edge` constraint holds the part at the
nearest side facing its edge, measured from the part's body: on an L
whose right half lies lower, `"edge": "top"` puts a part in the right
half against the step, not against the box's top (which is off the board
there). The body must lie alongside that side (not beyond its ends); on
a slanted side the distance counts from where the body's corner meets
it. `any` picks among the four the same way. On a rectangle with round
corners or cutouts, or on pieces that are all rectangles, edges are the
bounding box's (each piece's) as before. Cutouts are never edges.

**What stays the box's.** `region`, `keepout` and `place` with `x`, `y`
count from the top-left corner of the outline's bounding box (the same
as the polygon's `x`, `y` unless an extreme corner of it is rounded:
then the rounded shape's box). `place`'s `center` and halves are cut
from that box; its edge and corner words are `edge` constraints (a
corner is two: `top-left`, the nearest side facing up and the nearest
facing left), so on a polygon they mean its sides and corners, and so do
the corners `move_all` sends mounting holes to. `room` reports the box as
`outline_mm` and suggests rectangles; its usable area is the polygon's
area less the cutouts.

The automatic size (`"outline": {}`) and `shrink` are for rectangles: a
polygon or pieces have the size they are given (`area_factor`, `aspect`
and `shrink` with them are errors). `corner_radius` works with an
automatic size; cutouts need a size.

## A free dimension (`free_dimension`)

When one side of the board may be any length ("the height can change;
it only has to stay under 100 mm, where JLCPCB charges more"), say so in
the layout config instead of laying the board out at several sizes by
hand. `layout-kicad-board` then lays it out at a ladder of sizes and
keeps the smallest one that comes out clean:

```json
{"placer": {"constraints": "constraints.json"},
 "total_seconds": 6000,
 "free_dimension": {"edge": "bottom", "min": 34, "max": 60, "step": 2,
                    "prefer_below": [100], "band_mm": 10}}
```

| Key | Meaning |
| --- | --- |
| `edge` | The edge that moves: `bottom` or `top` (the height changes), `right` or `left` (the width). The opposite edge stays. |
| `min`, `max`, `step` | The board's extent along that axis to try, in millimetres: `min`, `min + step`, ... and `max`. Or `sizes`: a list. |
| `prefer_below` | Price thresholds. A size at or below one is preferred over any larger size; the search goes past it only when nothing at or below it is clean. A threshold inside the range is tried itself. |
| `accept` | What counts as clean: `pass_strict` (default): the layout's verdict is `complete` (exit 0: KiCad finds no unconnected item and no error that counts, every part placed, every constraint kept); `pass`: electrically good (KiCad finds no unconnected item and no error beyond the reference's count, every part placed; a missed soft constraint is allowed). The exit follows the rule chosen: a size clean by `pass` makes the search's verdict `complete` (exit 0) though its layout said `incomplete`, which `free_dimension.layout_status` keeps. Cosmetic findings never count. |
| `band_mm` | What moves with the edge (default 0.5: what lies on it). See below. |
| `probe_seconds`, `confirm_seconds` | The budgets (below). Keys of `free_dimension` in the layout config: the router config's own `probe_seconds` is another setting (the ladder's rung probes). |
| `parallel` | Probes side by side (default a quarter of the threads, at most 4, at most one per 3 GB of free memory). |

The outline must be a rectangle: the constraints' `outline` with a
`width` and a `height` (`corner_radius` is kept; a polygon, pieces,
cutouts or an automatic size are refused), or, without one, the board's
own Edge.Cuts as one `gr_rect` or lines along the sides of a box
(graphics inside it, cutouts, are left alone; arcs or slanted sides on
the outline are refused, exit 2). The size the board has is one size
like the others: `min` may be smaller.

**What moves with the edge.** Everything within `band_mm` of the moving
edge (or beyond it) moves with it; everything else keeps its place in
board coordinates:
- footprints by their origin (a fixed connector row at the bottom edge,
  its decoupling capacitors, the LEDs a generator put along it); a part
  the layout holds (below) also when its body lies at the edge (its side
  nearest the edge within 0.5 mm of it or beyond: a centre-origin
  connector, a module over the edge), whatever its origin; a tall held
  part that only reaches into the band stays. Parts the placer places
  again go by their origin;
- board texts, images, dimensions, circles and vias whole, by the side of
  their box nearest the edge (a text's box estimated from its font,
  length, justification and angle; a circle's from its radius): a label
  whose box reaches into the band moves with it although its anchor does
  not;
- lines, rectangles, polygons, tracks and zones point by point: a point
  in the band moves, so a pour over the whole board grows with it and a
  rule area at the edge moves whole. Where shrinking would carry the
  points in the band past points beyond it (a pour's notch just past the
  band), a zone is cut at the new edge instead (its outline and its
  fills; KiCad clips a pour to the board anyway), and a drawn shape (a
  silkscreen frame) would fold: that size cannot work (below);
- arcs (track arcs and drawn ones) by their ends: whole when both ends
  lie in the band, not at all when neither does, and when one does that
  end moves and the arc keeps its shape between its ends (its middle
  taken along by the similarity that maps the old ends to the new), so
  what is joined to either end stays joined;
- `region` and `keepout` boxes side by side along the free axis: a side
  in the band moves, so a box at the edge moves whole and one reaching
  from the edge across the board (a full-height strip) stretches with it
  and keeps covering its far end; `place` and `add` points by the point,
  whatever their `origin`: a box relative to the outline's corner that
  stays is written relative to the corner at that size (the corner moves
  when `top` or `left` is free);
- `edge` constraints and the words of `place` and `region` (`bottom`,
  `top-right`, `bottom-half`) are relative to the outline anyway.

Choose `band_mm` to take what belongs to the edge: on the fence board
the generator's bottom band (the header J1 1.3 mm from the edge, the
LED row 4.2 mm, its resistor networks and capacitors 8.2 mm) moves with
`"band_mm": 10`, and the parts 12 mm from the edge and farther stay, as
the generator itself lays out its 40 and 46 mm variants.

**Sizes that cannot work** are reported without a run:
- a held part or a rule area (a board zone that is a keepout, a `keepout`
  or `region` box, a `place` point) that lay inside the outline and would
  reach beyond it, and a held part carried by the edge that would land on
  a held part that stays;
- a drawn shape, text, circle, cutout (Edge.Cuts inside the board) or
  other board graphic that lay inside the outline and would end beyond
  it: the edge passing a label 1 mm in from it, an inner cutout just
  past the band (copper zones are cut instead, tracks go with the
  layout);
- a drawn shape that would fold, and a `keepout` or `region` box whose
  side at the edge would pass its far side.

The parts held are the ones the layout holds, decided by the same
function the placer and the keepout constraints use (`held_parts`, from
the lowering): the placer's `fixed` and `fixed_patterns`, constraints.json's
`fixed` and parts it places with `x` and `y`, and by default locked parts,
parts without a net and parts at the edge (a body over the outline, a
connector near it, a part in a designer's rule area), unless `free`, a
constraint naming them or `move_all` frees them. Infeasible sizes are
listed in `free-dimension.json` as `infeasible` with the reasons; when no
size is feasible the run is an input error (exit 2), and `verdict.json` and
`free-dimension.json` are written all the same, the table with every
size and its reasons.

**The search.** Results are not monotone in the size (the fence board at
34, 40 and 46 mm: 32, 1 and 2 unconnected), so the sizes are not
bisected. From the smallest size upward, `parallel` sizes at a time are
laid out at the probe budget, each a layout of its own with the same
seed. The first batch with a clean size stops the probes. Then the
search spends everything left of `total_seconds` on one full run at a
time (2026-10-21; it used to give the full run `confirm_seconds` at
most, so a search with a clean first probe used under half its budget):
- after a clean probe, the largest smaller size that came close (every
  part placed, KiCad's unconnected items and errors at most 4 together,
  `CLOSE_FINDINGS`) is laid out again with all that is left (`retry`):
  results are not monotone in the budget either;
- else the smallest clean size is laid out again (`confirm`);
- without a clean probe, when the budget or the ladder runs out, the size
  that came closest (fewest parts unplaced, then fewest KiCad unconnected
  items, then errors, then missed constraints, then the smaller size)
  gets it (`closest`).

A full run that ends well inside its budget leaves the rest to the next
(a retry, then the confirmation), as long as the rest is more than the
run it repeats had. The result is the smallest clean run (the larger
budget's at the same size), or else the closest.

**Threads, memory and what can differ.** The probes share the parent's
work rates (pinned for all), and each gets a fixed share of the threads
(`RAYON_NUM_THREADS` = threads / `parallel`) and of the memory
(`PCB_MAKER_MEMORY_MB` = an explicit `PCB_MAKER_MEMORY_MB`, else the
memory available when the search starts, / `parallel`), the last batch's
too, so that each child sizes its routers side by side for its share
and not for the whole machine (they all start at once and used to plan
for the same free memory). A layout's work, and so its board, does not
depend on threads or on how many routers run side by side; a size's
probe can differ between runs at another `parallel` through the
wall-clock guard (`wall_guard`, stretched by the pinned rate over the
measured one), which ends a layout that the machine slows far past its
work, and on a board so large that one router of a finer lattice does
not fit a probe's share of the memory: the ladder then skips that
lattice (its `skipped` says so), as it would under any explicit limit.
The full runs, alone, have the process's threads and memory.

**The budget.** `total_seconds` is the whole search's work.
`confirm_seconds` (default a third of it) is kept for the full runs:
probes start while the work spent and one more batch leave it;
`probe_seconds` (default a third of that, as the short tier's budget
scale) is each probe's. With the defaults that is six probes: the fence
example above (6000 s, neither key set) has probes of 667 s and keeps
2000 s, so 4000 s of probes, six of them (the fence report's run set
`probe_seconds` 500 and `confirm_seconds` 1500: nine).

**A reference.** An explicit `--reference` moves with each size as the
input does: its own rectangular outline's edge, the same band, by the
same distance (its board at each size in
`free-dimension/<size>mm-<kind>/reference/`), so that a finding the
input shares with it is at the same place at every size and still
counts as pre-existing. A reference whose outline is not a rectangle is
compared as given (the log says so).

**What comes back.** `<out-dir>` is the chosen run's layout, as without
the key (`result/`, `placed/`, `board-layout.json`, `verdict.json`, its
paths rewritten), so the next steps are unchanged. Beside it:
- `free-dimension.json`: the ladder, the budgets, the reason and every
  run (`size_mm`, `kind` `probe`, `retry`, `confirm`, `closest` or `infeasible`,
  `budget_seconds`, `status`, `clean`, `kicad_unconnected`, `errors`
  and `errors_by_type` under the strict rule, `errors_count_rule`,
  `strict`, `constraints_missed`, `unplaced`, `work_seconds`,
  `wall_seconds`, `chosen`, `reasons`, `directory`), and `untried`:
  sizes the budget left;
- `free-dimension/<size>mm-<kind>/`: each run's project (`input/`, the
  board at that size), `layout.json`, `layout.log` and `layout/`;
- every JSON report of the chosen run (the verdict, `board-layout.json`,
  `placed/board-placer.json`, `result/board-router.json`, ...) names its
  new place: its paths are rewritten by whole path components, inside
  sentences too (`next_actions`, reasons), never a file beside the run's
  directory (`layout.log`);
- the verdict's `free_dimension`: `chosen` (mm), `kind`, `clean`,
  `probes` (runs made), `reason` (which smaller sizes failed and how,
  a threshold exceeded), `edge`, the search's `work_seconds` and
  `wall_seconds` (the verdict's own are the chosen run's), `report`.
  When no size is clean, the first of `next_actions` names the size
  that came closest and what blocked it; the rest are that layout's.

The exit code is the chosen run's. `place-kicad-board` reads only the
layout config's `placer` object and places at the size given.

## Parts the schematic does not have (`add`)

A board often carries parts its schematic lacks: mounting holes plated on
GND, a second footprint for another switch, a test point. An agent need
not draw them into the schematic: an `add` entry in the project's
`constraints.json` says what to put on the board, and
`import-kicad-netlist` (run again into the same directory, which keeps
`constraints.json`) puts it there.

```json
"add": [{"part": "H1", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad", "nets": {"1": "GND"}, "x": 3.5, "y": 3.5},
        {"part": "H2", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad", "nets": {"1": "GND"}},
        {"part": "MX37_1", "from_board": true},
        {"part": "TP_1", "from_board": true, "board_reference": "REF**", "x": -10, "y": 22.5, "origin": "absolute"}]
```

| Field | Meaning |
| --- | --- |
| `part` | The new part's reference. A part of the schematic with that reference is an input error (exit 2). |
| `footprint` | `Library:Name`, looked up in the project's libraries as the schematic's footprints are; not found, it is reported in `missing` (exit 1). |
| `from_board` | `true`: the footprint `part` (or `board_reference`) of the board given with `--footprints-from`, in that board's version; where the reference repeats there (mounting holes all called `REF**`), the one at `x`, `y` with `"origin": "absolute"`. Without `--footprints-from` an input error. Give `footprint` or `from_board`, not both. |
| `nets` | Pad number to net name (`{"1": "GND"}`; a mounting hole's pad may be numbered `""`). Pads not named have no net. Without `nets`, a `from_board` part keeps that board's nets and a library footprint has none. A net the schematic does not have is allowed and named in the report. A pad number the footprint does not have is an input error. |
| `x`, `y`, `angle`, `origin` | Where it goes, as `place` says it: `origin` `board` (default, from the outline's corner; the layout puts it there) or `absolute`. Without `x` and `y` a `from_board` part stays where that board has it, on its side, locked, and a library footprint is placed by the placer (mounting holes go to the corners under `move_all`). |
| `side` | `front` (default for a library footprint) or `back`; a `from_board` part keeps that board's side. |

The parts are written `board_only` ("not in schematic"), so that KiCad's
schematic parity check leaves them out; their pads are routed like any
other. The import reports each in `added`. The layout refuses an `add`
part the board does not have (an input error that says to import again).

## Not supported yet

- Choosing sides automatically: parts stay on their side unless `back` or
  `front` names them.
- Routing intent: differential pairs, length matching. Track widths and
  clearances come from the project's KiCad net classes, or from the router
  config's `net_classes` ([agents.md](agents.md)).

## From a netlist to a board

A board fresh from a netlist import (every footprint at one point, no
outline; KiCad's "Update PCB from schematic", kinet2pcb, SKiDL) can be
laid out from the constraints alone.

For example, KiCad's complex-hierarchy demo, stacked and without its outline,
with these constraints:

```json
{"version": 1,
 "outline": {"width": 100, "height": 80, "x": 0, "y": 0},
 "edge": [{"part": "P101", "edge": "left"}, {"part": "P102", "edge": "left"},
          {"part": "P201", "edge": "right"}, {"part": "P202", "edge": "right"},
          {"part": "P301", "edge": "right"}, {"part": "P302", "edge": "right"}],
 "near": [{"part": "C101", "part_of": "U101", "max_mm": 3}]}
```

The result: every constraint holds, all 50 connections are routed with
2 vias, and KiCad finds no copper error (5 minutes).

## Example

`benchmarks/fence/esp-usb-placement`. The task: an ESP32-C3 module flush in
the top-left corner, its antenna beyond the top edge; the USB-C connector
below it on the same edge; the regulator between them. With its
`constraints.json`, the placer puts U3 exactly where the designer put it
(56.8, 52.9). It meets every hard constraint and the distance limits for U2,
C3 and C4. It reports two limits as missed:

- **C5/C6 within 6 mm of U3 pin 3.** The pad sits 8.4 mm inside the
  module's courtyard, so no part on the board can come that close; the
  designer's own placement misses it by the same amount.
- **J4 at most 3 mm below U3.** This conflicts with C5/C6 sitting between
  them.

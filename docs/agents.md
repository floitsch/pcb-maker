# pcb-maker for agents

You have a KiCad schematic, or a board with footprints and nets from
"Update PCB from Schematic", SKiDL or a generator. pcb-maker places the
parts, routes the board, tells you precisely what it achieved, and writes
what a fab needs to make and assemble it.

## 0. From a schematic

```sh
pcb-maker import-kicad-netlist <project>/<name>.kicad_sch <project-dir> <board-id> [--layers 4]
```

does what KiCad's "Update PCB from Schematic" does for a new board:
- every symbol's footprint from the project's and the user's
  `fp-lib-table` (KiCad's own libraries without one), with its fields (an
  `LCSC` field ends up in the BOM) and its DNP and BOM flags;
- pads on their nets, footprints linked to their symbols, so KiCad's
  schematic parity check passes;
- the schematic, project file and the project's own libraries copied
  next to the board.

A KiCad netlist (`.net`, from `kicad-cli sch export netlist`) works in
place of the schematic; its footprints come from the libraries of the
netlist's folder and of the project the netlist names as its source. A
file that is not a KiCad netlist is an input error (exit 2). Footprints
from KiCad 5 and 6 libraries (`module`, `fp_text reference`, arcs given by
centre and angle) are brought to today's form, so KiCad loads the board
and reads the schematic's reference, not the library's. Libraries outside
the project folder (`${KIPRJMOD}/../lib`) are used but not copied. All
footprints sit at one point and there is no outline. Next to the board it writes, unless they exist:
- `constraints.json`: `"move_all": true`, `"outline": {}` (sized from the
  parts), and the plug-in connectors (USB, RF, audio and barrel jacks,
  D-sub, network, video, card edges, terminal blocks) on `"edge": "any"`,
  facing out. Edit it: it is where what the user wants goes.
- `layout.json`, which uses it, so that
  `pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto <project-dir>/layout.json`
  lays the board out.

**A keyboard: `constraints.starter.json`.** When the netlist holds a key
matrix, the import also writes `constraints.starter.json` beside
`constraints.json` (rewritten on every import; `constraints.json` never
is). From the netlist alone it finds the switches (footprint or symbol
named `SW`, `KEY`, `MX`, `Choc`, `hotswap`, or a two-pin part on a row and
a column), each key's diode (a two-pin `D` part on a net with that switch
alone), the rows and columns (a switch's two nets, directly or through its
diode: rows by their names, `ROW0`, `r2`, `col_3`, else the side with
fewer nets) and the halves of a split keyboard (two matrices no switch
joins, or columns named for a hand, `col_L1` and `col_R1`). The file is
the import's starter constraints plus:
- a `row` per matrix row, its keys in column order at the key pitch
  (Choc 18 x 17 mm, else 19.05 mm, or the switch body where it is wider);
  rows over the same columns, one after the other, are one grid
  (`columns`, `row_pitch_mm`); keys larger than the pitch (1.5u, 2u, an
  ISO enter) stay out of the rows, `near` their neighbour, the keys after
  them `relative` right of the keys before; a row whose columns have a gap
  (thumb keys of both hands on one row) is a row per piece;
- each row `relative` below the one before (short rows, the thumb keys,
  up to 10 mm below);
- each diode `near` its key (3 mm);
- per half, a `region` in words (`"at": "left-half"`) with its rows and its
  controller (the part with the most pins on the half's lines);
- `comment`: what was inferred and what needs your decision (key pitch and
  stagger, thumb clusters, the controller's edge, the outline, the switches
  outside the matrix).

Copy it to `constraints.json` and edit it there. The report's `starter`
says what was found: `matrix` (row and column nets), `switches`,
`diodes`, `halves` (each with its rows, columns, switches, controller),
`unassigned` (switch-like parts outside the matrix: reset and power
switches), `wide`, `alternates` (a second footprint on the same key), or
`why` no file was written (keys wired each to its own pin: no matrix).
Urchin (two 4 x 5 halves) from the starter alone plus the walkthrough's
230 x 95 mm outline: 42 of 42 constraints kept, 68/68 routed, KiCad 0
unconnected, exit 0 (`"total_seconds": 30`).

The report lists what needs a look:
- `missing`: footprints not found in any library (the board leaves them
  out), or unreadable, each with where it was looked for: no library of
  that name (add it to the project's `fp-lib-table`, or put its `.pretty`
  folder in the project), or the library without that footprint; or give
  a board that has them (`--footprints-from`, below). Then
  import again into the same directory: the board is written anew,
  `constraints.json` and `layout.json` are kept;
- `substituted`: footprints found in another library or under KiCad 9's
  name (`DSUB-25_Female` is now `DSUB-25_Socket`);
- `unmatched_pins`: symbol pins on a net that the footprint has no pad
  for (a library that changed since the schematic was drawn). A pin on
  KiCad's own `unconnected-(...)` net (a no-connect pin, a module's pads
  the footprint leaves out) loses nothing; KiCad's schematic parity check
  still reports it as `net_conflict` ("No pad found for pin"), which the
  verdict counts in `schematic_parity_issues` and does not judge;
- `clearances`: footprints whose own pads are closer than the board's
  clearance (a solder jumper's), given their pads' gap as their own
  clearance, as designers do;
- `added`: the parts of `constraints.json`'s `add` (below).

**A board to take footprints from: `--footprints-from <board.kicad_pcb>`.**
Designers keep footprints in personal libraries that do not travel with
the project, and libraries change after a board is made (a courtyard that
grew, a hotswap socket redrawn). When you have a board with these parts
(the designer's, an older revision, a colleague's), give it:

```sh
pcb-maker import-kicad-netlist <project>/<name>.kicad_sch <project-dir> <board-id> --footprints-from old/<name>.kicad_pcb
```

- A footprint no library has is taken from that board: the symbol's own
  footprint (by the symbol's path, else its reference), else any footprint
  of that name. It is brought back to library form as KiCad's "Export
  Footprints to Library" does (flipped to the front, rotation and position
  removed, nets dropped) and written to a library in `<project-dir>`
  (`<library>.pretty` under the schematic's library name and named in the
  project's `fp-lib-table`; `from_board.pretty` when a library of that
  name exists but lacks the footprint), so the project is complete.
  Reported in `footprints_from_board`. A symbol without a footprint takes
  the board's footprint of it too.
- Where the board has the symbol's footprint, **the board's version is
  used over the library's**: the board is what was made and checked, the
  library is what changed since. The count is `board_versions`;
  `drifted_footprints` lists the ones whose library version differs from
  the board's by more than a fifth on a side or in pad count, with both
  sizes (`lib:SW (64 parts: SW1, ...): library 15.93 x 12.20 mm, 10 pads;
  board 12.85 x 11.60 mm, 6 pads`). Where the board's footprint has another
  name than the schematic's (the board was changed after the schematic, or
  the schematic after the board), the board's is used under its own name
  and listed in `substituted`; the copied schematic names it too. Want the
  library's versions instead? Import without `--footprints-from`.
- A symbol's own footprint (and a `from_board` part's) **keeps the board's
  ids** (the footprint's, its pads' and drawings'), as KiCad's "Update PCB
  from Schematic" does: the project's DRC exclusions name items by them,
  so a finding the designer excluded (two courtyards that overlap by
  design, a key socket over the edge) stays excluded on the new board
  where the part stays where the designer put it. A copy of a footprint
  (another part of that name) gets ids of its own.
- `missing` then says when the board lacks a part as well.

**Parts the schematic does not have** (mounting holes plated on GND, a
second footprint for another switch): an `add` entry in
`<project-dir>/constraints.json`, then import again into the same
directory. `{"part": "H1", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad",
"nets": {"1": "GND"}, "x": 3.5, "y": 3.5}` adds a library footprint there;
`{"part": "H1", "from_board": true}` takes the `--footprints-from` board's
H1 with its nets, where that board has it. The parts are `board_only` (the
parity check leaves them out) and are routed like the others;
[constraints.md](constraints.md#parts-the-schematic-does-not-have-add).

## 1. Look at the board

```sh
pcb-maker describe-kicad-board <project-dir> <board-id> > board.json
```

The file lists what constraints refer to:
- the outline's box and the copper layers (`outline` is null without
  one; `from_footprints: true` when the board draws no Edge.Cuts itself
  and the box is only footprints' slots and cut-outs, as on a fresh
  import: give the board its size with `outline` in `constraints.json`);
- every footprint with reference, value, library name, position, side
  (where its pads put it), `layer` (the footprint's own, `F.Cu` or `B.Cu`:
  what `back` and `front` set; a key socket under its switch is a front
  footprint with its pads on the back), size, whether it is through-hole
  or locked, its `uuid` (which footprint
  an entry for a repeated reference means, [constraints.md](constraints.md)),
  and its pads with their nets;
- every net with its pads (`REF:PAD`, the form `pin_of` takes) and its track
  width and clearance;
- `swap_candidates`: parts whose general-purpose pins the layout could
  reassign (see pin swapping below).

## 2. Say what you want: `constraints.json`

```json
{"version": 1,
 "move_all": true,
 "outline": {"width": 60, "height": 40},
 "edge": [{"part": "J1", "edge": "left", "flush": true}],
 "near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]}
```

Full reference: [constraints.md](constraints.md).
- **`move_all`** places every part, except locked ones and those named in
  `fixed`. Use it for a board straight from a netlist. Mounting holes no
  constraint names go to the corners.
- **`place`** says where a part goes in words (`"at": "top-left"`,
  `"center"`, `"front"` with `device_front`) or exactly (`x`, `y`,
  `angle`, for a part that must match an enclosure).
- **`outline`** sets the board size, or its shape: a `polygon`, round
  corners (`corner_radius`), holes (`cutouts`) or several `pieces`
  ([constraints.md](constraints.md#outlines-that-are-not-rectangles)).
  On a polygon, `edge` means the nearest side of the polygon facing that
  way.
- **`edge`, `region`, `keepout`, `rotation` and `fixed`** decide where parts may be;
  **`back`/`front`** on which side.
- **`hollow`** lets parts sit inside a big part's outline (a shield, a
  module above parts): only its pads block. Parts with holes still stay
  out of its courtyard's shapes, as KiCad forbids holes inside a
  courtyard.
- **`near`, `group` and `relative`** keep parts together; **`apart`** keeps
  them away from each other; **`row`** puts parts in a line at a pitch (LED
  bars, key rows). A row moves as its first part: other constraints name
  that part (and then hold the whole row: `"below": "SW1"` is below the
  grid), except that `near`'s `part_of` and `pin_of` may name any key of
  it (a diode at its switch).
- **`overhang`** lets a module's antenna reach beyond an edge.

Unknown keys, parts or pads are errors, not silently ignored.

### Cookbook: from what people say to constraints

| They say | Write |
| --- | --- |
| "The buttons are at the front, the front is the left side" | `"device_front": "left"`, `"place": [{"part": "SW1", "at": "front"}]` |
| "Power jack at the back" | `"place": [{"part": "J2", "at": "rear"}]` (with `device_front`) |
| "USB-C on the bottom edge, plug from outside" | `"edge": [{"part": "J1", "edge": "bottom", "flush": true, "opening_outwards": true}]` |
| "The connectors on the edge, whichever suits" | `"edge": [{"part": "J1", "edge": "any", "flush": true, "opening_outwards": true}, ...]` |
| "Mounting holes 3.5 mm in from each corner" | `"place": [{"part": "H1", "x": 3.5, "y": 3.5}, ...]` |
| "Four M3 holes on GND" (the schematic has none) | `"add": [{"part": "H1", "footprint": "MountingHole:MountingHole_3.2mm_M3_Pad", "nets": {"1": "GND"}, "x": 3.5, "y": 3.5}, ...]`, then import again |
| "Use the footprints of the old board" | `import-kicad-netlist ... --footprints-from old.kicad_pcb` (missing footprints from it, its versions over the libraries'; `drifted_footprints` says what differs) |
| "Keep the old board's extra parts" | `"add": [{"part": "MX37_1", "from_board": true}, ...]` with `--footprints-from` |
| "The connector must match the enclosure cut-out" | `"place": [{"part": "J3", "x": 12, "y": 0, "angle": 90}]` |
| "Status LED in the top-right corner" | `"place": [{"part": "D1", "at": "top-right"}]` |
| "Eight LEDs in a row, 5 mm apart, along the top" | `"row": [{"parts": ["D1", "D2", ..., "D8"], "pitch_mm": 5}]`, `"place": [{"part": "D1", "at": "top"}]` |
| "It's a keyboard" (a key matrix in the schematic) | copy `constraints.starter.json` to `constraints.json`, give it an `outline`, then decide what its `comment` asks (pitch, stagger, thumbs, the controller's edge) |
| "This half on the left of the board" | `"region": [{"parts": ["SW1", "U1"], "at": "left-half"}]` (`right-half`, `top-half`, `bottom-half`, `center`: on any outline) |
| "A 4 x 4 key matrix, 19.05 mm pitch" | `"row": [{"parts": ["SW*"], "pitch_mm": 19.05, "columns": 4}]` |
| "Each key's diode right at its switch" | `"near": [{"part": "D2", "part_of": "SW2", "max_mm": 3}, ...]` (one per key; the keys may be in a row) |
| "The thumb keys below the grid" | `"relative": [{"part": "SW16", "below": "SW1", "max_gap_mm": 10}]` (`SW1`, the row's first key, stands for the grid) |
| "WiFi antenna over the board edge" | `"edge": [{"part": "U1", "edge": "top", "flush": true, "overhang": {"edge": "top"}}]` |
| "Decoupling caps at their IC's supply pins" | `"near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]` (`REF:PAD`, the pad number; `describe-kicad-board` lists each net's pads) |
| "Keep the temperature sensor away from the regulator" | `"apart": [{"parts": ["U4"], "from": ["U2"], "min_mm": 10}]` |
| "Keep the power supply together, bottom right" | `"group": [{"parts": ["U2", "L1", "C1?"], "max_mm": 4}]`, `"place": [{"part": "U2", "at": "bottom-right"}]` |
| "Nothing under the display window" | `"keepout": [{"x": [10, 40], "y": [5, 25]}]` |
| "No copper under the antenna" | `"keepout": [{"x": [...], "y": [...], "copper": true}]` |
| "Battery holder on the back" | `"back": ["BT1"]` |
| "A HAT: the Pi's outline is fixed, parts go inside it" | `"fixed": ["J5"]`, `"hollow": ["J5"]` |
| "As small as possible" | `"outline": {"shrink": true}`: the smallest board a quick route completes (each size tried costs a placement and a route) |
| "100 x 60 mm with 3 mm round corners" | `"outline": {"width": 100, "height": 60, "corner_radius": 3}` |
| "It must fit the enclosure: an L, the right part 30 mm lower" | `"outline": {"polygon": [[0, 0], [115, 0], [115, 30], [230, 30], [230, 125], [0, 125]]}`; `"edge": "top"` for a part in the right part puts it against the step |
| "A window for the display, nothing in it" | `"outline": {"width": 80, "height": 50, "cutouts": [{"x": [20, 60], "y": [10, 35]}]}` (relative to the outline's corner; a `polygon` works too) |
| "The sensor board breaks off the main board" | `"outline": {"pieces": [{"width": 60, "height": 40}, {"width": 15, "height": 40, "x": 65}]}`; parts wired to a fixed part on a piece stay on it |

Router side (the file passed in place of `auto`):

| They say | Write |
| --- | --- |
| "Ground plane on the bottom" | `"add_pours": [{"net": "GND", "layers": ["B.Cu"]}]` |
| "Power traces 0.8 mm wide" | `"net_classes": [{"name": "Power", "nets": ["VBUS", "+5V", "GND"], "track_width_mm": 0.8}]` |

## 3. Run

```sh
echo '{"placer": {"constraints": "constraints.json"}}' > layout.json
pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto layout.json
```

- **`auto`** takes the design rules (net classes, clearances, widths,
  vias) from the project's `.kicad_pro`.
- **Only routing?** When the placement is already final, use
  `pcb-maker route-kicad-board <project-dir> <board-id> <out-dir> auto`.
- **A designer's board as the reference.** When the board comes from a
  finished design, `--reference <its-project-dir>` makes its own DRC
  errors pre-existing (see 4).
- **Router options.** Pass a JSON file instead of `auto`, for example
  `{"seeds": 4}`. The rules still come from the project.
  - `seeds`: route that many ways in parallel and keep the best: fewest
    opens, then vias, then copper. Worth it on idle cores; best of 4 took
    PIC from 2 vias to 0.
  - `pours`: `auto`, `connect` or `tracks`.
  - `add_pours`: planes to add before routing, such as
    `[{"net": "GND", "layers": ["B.Cu"]}]`. Each covers the board outline;
    pads connect to it, and islands are stitched with vias. A layer where
    that net already has a zone is skipped.
  - `automatic_planes` (default true): a board with four or more copper
    layers and no pours gets ground as a plane on the first inner layer
    and the supply rail with the most pads on the last inner layer, as a
    designer would lay it out. `add_pours` replaces this.
  - `exclusive_planes`: an inner-layer pour over the whole board is a
    plane. Left unset, the router first keeps other nets' tracks out of it
    (it stays whole) and, if connections stay open, then lets them cut it
    as designers do (`plane_cut_cost`, default 3, prices that). `true` or
    `false` picks one.
  - `net_classes`: track widths and clearances for named nets, such as
    `[{"name": "Power", "nets": ["VBUS", "+5V", "GND"], "track_width_mm": 0.6,
    "clearance_mm": 0.3}]`. The fields are `via_diameter_mm` and
    `via_drill_mm` besides those two. Nets take KiCad wildcards (`*`, `?`)
    and may leave out the sheet path's leading `/`. The first class naming
    a net wins. The classes go into the result's `.kicad_pro` ahead of the
    project's own, so KiCad and its DRC use the same rules. Without them,
    net classes come from the project.
- **Router options in layout mode.** The same file goes in the router
  config position:
  `pcb-maker layout-kicad-board <dir> <id> <out> router.json layout.json`.
  - `pin_swaps`: see below.
- **Pin swapping.** `"swappable": [{"part": "U1", "pins": ["GPIO*"],
  "except": ["GPIO0"]}]` in the layout config lets pcb-maker reassign
  interchangeable pins (pad functions or numbers, globs); `"pin_swaps":
  "pin-swaps.json"` takes the full format (resistor networks, groups
  across parts, restrictions). The schematic is rewritten to match
  ([pin-swap.md](pin-swap.md)). `describe-kicad-board` lists
  `swap_candidates`: parts (not connectors) with three or more connected
  general-purpose pins (`GPIO12`, `IO5`, `PA3`, `P0.13`) and the globs for
  them. Ask whether the firmware can use any of them for any signal (a
  GPIO matrix, PIO, bit-banged buses) and which pins are special (boot
  straps, a UART the bootloader uses, ADC-only pins): those go in
  `except`.
- **Output.** `<out-dir>/result/` holds the finished project, copied, with
  the placement and the copper. `<out-dir>/placed/placement.html` animates
  the placement.
- **Iterate quickly.** Work in three steps:
  1. Check the constraints alone, which takes seconds:
     `pcb-maker place-kicad-board <dir> <id> <out> <dir>/layout.json`
     (the `placer` object of a layout config is used; a file holding only
     that object works too). What cannot fit is said at once, with exit
     2: a `region` smaller than its part, a `row` longer than the outline
     (as it may turn), parts held at one edge that need more length than
     the edge has. Parts that need more area than a side has even with
     their tightest bodies, or that cannot lie side by side in strips of
     the board, are a no in seconds (exit 1, `"overfull": true` in
     `board-placer.json`). Either way `verdict.json` has `room`: what is
     missing, and an outline to try. A board that is merely crowded is
     the slow case: the placer tries looser levels before it reports
     parts as unplaced, up to the layout's placement budget (600 s of
     work per seed; urchin's 80 parts on 80 x 40 mm without rows: about
     seven minutes); `PCB_PLACER_WORK_SECONDS=60` gives a quicker answer.
     `<out>/board-placer.json` reports every constraint, `unplaced`,
     `hints`, `utilization` and `room`. A directory holding only the
     verdict of such an input error is taken again by the retry.
  2. Make a draft: `{"moves": 0, "placer": {...}}` in `layout.json` places
     and routes once, skipping the router-driven part moves. Those moves
     take most of a layout's time.
  3. Make the final run with the defaults. It makes up to 40 moves, for
     at most 600 s while connections are open and 60 s
     (`polish_seconds`) once everything is routed.
  - **The budget.** `total_seconds` in `layout.json` (default 1500) is
    the layout's whole budget in seconds of work (an idle machine's
    seconds, so a run does the same on a loaded one): the placement
    race stops at 40 % of it, the moves at 65 %, the final route gets
    the rest. A small board needs far less: `"total_seconds": 30` lays
    out a 68-net keyboard in about 15 s. The wall clock guards it at 1.1
    times (exit 3, `cut`); `PCB_ROUTER_NO_WALL_GUARD=1` leaves it to the
    work alone.

## 4. Read the verdict: exit code and `<out-dir>/verdict.json`

Every run of `route-kicad-board`, `layout-kicad-board` and
`place-kicad-board` ends with an exit code and a verdict that say the
same. `pcb-maker --help` and `pcb-maker <command> --help` list them too.

| Exit | `verdict.status` | Meaning | What to do |
| --- | --- | --- | --- |
| 0 | `complete` | The run finished and the result passes. | Use it. |
| 1 | `incomplete` | The run finished and wrote its result, but connections are open, there are new DRC errors, constraints are missed or parts found no place. | Read `next_actions`. |
| 2 | `usage_error` | Bad arguments or input: a missing file, invalid JSON, an unknown key or part, an output directory that exists. Nothing was routed. | Fix the input the message names. |
| 3 | `cut` | Incomplete, and the wall clock (not the work) cut the run short (`cut`): the result depends on the machine's load. | Rerun on a less loaded machine or with a larger budget. |
| 4 | `internal_error` | An internal error or a panic. | Report it with the input. |

A process killed by a signal (a harness's timeout, out of memory) has no
verdict; its exit status is the signal's (128 + n in a shell). Errors
and incomplete results print one line on stderr.

**The pass rule** is the benchmarks' (`benchmarks/agent-tasks/run.py`):
KiCad's own DRC on the result decides, not the router's count. A result is
complete when KiCad reports no unconnected item and no error beyond the
ones the reference board already has, counted per type. Warnings,
excluded findings and cosmetic types (`silk_*`, `text_*`, library
findings, `missing_courtyard`, `isolated_copper`) do not count. A layout or
placement must also place every part and keep every constraint. Without
KiCad's check (`PCB_SKIP_NATIVE`), the router's own count decides.

**The reference board** is where pre-existing findings come from (a
fiducial near a hole, a rule the designer tightened later): `--reference
<project-dir>` names it. Route mode defaults to the input project (its
board as given); layout mode to none, since its input is not a placed
board. `--reference none` counts every error. The reference is checked
(one more KiCad DRC, kept as `reference-drc.json`) only when the result has
errors that count.

`<out-dir>/verdict.json` is written for every run that got as far as an
output directory, also for an error (with `error`) or when the layout
stopped because parts found no place (then there is no `board-layout.json`).
A finished run also carries it as `verdict` in its report:

| Field | Meaning |
| --- | --- |
| `status`, `exit_code`, `reason` | As above; `reason` is one line. |
| `open_connections`, `unconnected_terminals` | The router's own count of unfinished connections and unreached pads. |
| `kicad_unconnected_items` | KiCad's count, which decides (`null`: KiCad did not check). |
| `new_findings` | KiCad errors per type beyond the reference's: the run's own. |
| `preexisting_findings`, `preexisting_from` | Errors per type the reference board has too, and its project. |
| `cosmetic_findings`, `warnings`, `schematic_parity_issues` | Counted, not judged. |
| `schematic_parity_by_type` | `schematic_parity_issues` by KiCad's type (`net_conflict`, `missing_footprint`, ...). When every one is a pin the footprint has no pad for (`net_conflict` "No pad found for pin", a no-connect pin), `next_actions` says there is nothing to do. |
| `starved_thermals`, `internal_violations` | Pads short of thermal spokes; the router's own clearance check. |
| `constraints_missed`, `unplaced` | Placement constraints not kept (with mm), parts without a legal place. |
| `cut` | What the wall clock cut short (`deadline`, `negotiation guard`, `budget guard`, `ladder`, `placement`, `layout`). |
| `work_seconds`, `wall_seconds` | The work done (seconds of an idle machine, what budgets count) and the wall clock. |
| `kicad_check_work_seconds` | Layouts: the part of `work_seconds` that is KiCad's checks inside the layout (the board kept, the final ladder's attempts, the neck repair), each counted at a cost modelled from the board, the same whether the DRC cache answered it or not. |
| `skipped` | Steps of the router's attempt ladder that did not run while the board was incomplete, with why (`the narrow-signal step (signals at 0.15 mm) did not run: the ladder's budget was used (ladder_budget_seconds)`). |
| `outputs` | Absolute paths: `project`, `board`, `drc`, `report`, `verdict`, `placement`. |
| `next_actions` | What to do next, factual: where nets fought, which pads no track enters, which findings are new, which budget ran out. |
| `drc_cache` | `{calls, hits}`: KiCad DRC runs the process asked for (its attempts, the reference) and how many the DRC cache answered (see below). |
| `room` | Placement and layout (also an input error that a constraint needs more room than the outline has): the room as numbers, below. Absent for a route. |

`room` says how full the board is and how large it would need to be, so
that an agent enlarges it in one step:

| Field | Meaning |
| --- | --- |
| `outline_mm` | The outline's box, width and height. |
| `usable_mm2` | Per side (front, back): the outline's area (a polygon's own, less its cutouts) less the band along the edge bodies keep free and less the fixed parts' bodies. |
| `parts_mm2` | Per side: the movable parts' bodies (courtyards, or the body box) with the spacing around them; a row is one body. |
| `utilization` | `parts_mm2` over `usable_mm2`, per side. |
| `target_utilization` | 0.7: above it a side is called full and placement gets hard. |
| `missing_mm2` | Per side: the usable area missing for the parts at the target. |
| `smallest_outline_mm` | The smallest outline (at the current aspect, unless large parts need another) that holds every side's parts at the target by area, its large parts side by side in strips across it (as full as the target), the largest part, every row and every edge's parts. An estimate from sizes: the placement may need more. |
| `suggested_outline_mm` | Only when parts found no place or the outline failed a check: the larger of the current outline and `smallest_outline_mm` per side, or, where area is not what is missing, a quarter more area. `next_actions` names it ("try an outline of 118.0 x 107.5 mm ..."). |
| `limits` | What bounds the outline besides area: rows, edges, parts larger than the board. |

Urchin (both keyboard halves, 80 parts) on 80 x 40 mm, its two 5 x 3 key
grids as rows: exit 2 in 0.04 s, with

```json
{"outline_mm": [80.0, 40.0], "usable_mm2": [3080.0, 3080.0], "parts_mm2": [1419.0, 3038.8],
 "utilization": [0.461, 0.987], "target_utilization": 0.7, "missing_mm2": [0.0, 1261.2],
 "smallest_outline_mm": [118.0, 107.5], "suggested_outline_mm": [118.0, 107.5],
 "limits": ["the row of SW1 (15 parts) is 88.2 x 43.7 mm", "the row of SW18 (15 parts) is 88.2 x 43.7 mm"]}
```

(118 x 107.5 mm then places every part; two `near`/`relative` entries
written for the halves side by side miss.) On the 230 x 95 mm board the
walkthrough uses, the layout's verdict has `utilization` [0.066, 0.141],
`missing_mm2` [0, 0] and `smallest_outline_mm` [213.5, 54.5]: the two
grids side by side.

**KiCad's DRC is cached.** Every DRC (15-25 s and 2.2 GB on a large
board) is kept in `$XDG_CACHE_HOME/pcb-maker/drc/` (`~/.cache/pcb-maker/drc/`
without it) under a hash of everything it reads: the board's bytes and file
name, the KiCad files of the project (`.kicad_pro`, `.kicad_dru`,
schematic sheets, project libraries and library tables), the user's global
library tables, `KICAD*` environment variables, the `kicad-cli version` and
the DRC's arguments. A rerun on unchanged inputs (the reference board, an
attempt that ends with the same board, a second run of the same task)
takes the stored report: KiCad's exact bytes, validated as a fresh report
is; only its `date` is the first run's. `PCB_MAKER_CACHE_DIR=<dir>` keeps
the cache in `<dir>/drc/`, `PCB_MAKER_NO_CACHE=1` turns it off. Processes
side by side share it safely (entries are written whole, then renamed);
it keeps at most 2000 entries and 2 GB, dropping the least recently used.
KiCad's own libraries are not hashed (only the `lib_footprint_*`
warnings, which never count, read them); after changing them without a new
KiCad version, delete the directory. The stderr line `board check: ...`
and `drc_cache` in the verdict count the calls and hits.

**KiCad's DRC runs without the global libraries.** kicad-cli's DRC
otherwise loads every footprint library of the global `fp-lib-table`
first (about 2.7 s, 9 CPU s and 2 GB per call, for the `lib_footprint_*`
warnings alone), so pcb-maker runs it with `KICAD_CONFIG_HOME` set to
`<cache root>/kicad-config/` (the root as for the cache: `PCB_MAKER_CACHE_DIR`,
else `$XDG_CACHE_HOME/pcb-maker`, else `~/.cache/pcb-maker`), which holds
empty `fp-lib-table` and `sym-lib-table`. Every finding but the
`lib_footprint_*` warnings is the same. The user's configuration is kept
when the project sets a `lib_footprint_*` severity to `error`, and with
`PCB_MAKER_KICAD_USER_LIBS=1`. ERC runs the same way (its library rules
`lib_symbol_issues`, `lib_symbol_mismatch`, `footprint_link_issues`):
these three warnings depend on the libraries installed, so
`verification.json` leaves them out of `erc_violations` and counts them by
type in `erc_library_warnings`.

The other commands exit 0 when done, 1 when their result is incomplete
(`repair-kicad-silkscreen` with findings left, `import-kicad-netlist` with
footprints missing), 2 on usage or input errors and 4 on internal errors.

### The layout's report: `<out-dir>/board-layout.json`

| Field | Meaning |
| --- | --- |
| `routed.routed_connections` / `routable_connections` | Connections made / needed. |
| `routed.unconnected_terminals` | Pads the router could not reach. |
| `routed.native` | KiCad's raw counts: `complete`, `drc_design_violations`, `selected_net_unconnected_items`, `schematic_parity_issues`. They count every finding of any severity, silkscreen warnings included (`repair-kicad-silkscreen` fixes most of them), so `complete` is often false on a board that passes: `verdict` decides. |
| `routed.vias`, `routed.length_mm` | Cost of the routing. |
| `constraints[]` | Every constraint: `kind`, `part`, `other`, `satisfied`, `violation_mm`. The ones the placer keeps by construction are listed too: `back` and `front` (a part on its side), `rotation` and `opening_outwards` (a part at an allowed angle). |
| `routed.diagnostics.dead_pads[]` | Pads no track of their net's class can enter (`pad`, `net`, `at`, the objects `near` it): a class too wide for the pad pitch, or something covering the pad. |
| `routed.diagnostics.conflicted_nets` | Nets still fighting for room when the router gave up. |
| `routed.diagnostics.hot_spots[]` | Where they fought longest (`layer`, `at`, `history`), worst first: give these areas more room. |
| `constraint_warnings` (in `placed/board-placer.json`) | Inputs accepted but not acted on. |

## When it does not work out

- **`placement is not legal (unplaced [...]); hints`.** No position
  satisfies the hard constraints for these parts. The layout stops before
  routing (exit 1) and writes no `board-layout.json`: `verdict.json` has
  `unplaced`, `error` and the hints in `next_actions`. The hints after the
  semicolon say why: a side that is more than 70 % full (parts' bodies
  with spacing against the board area), or parts held by an `edge`,
  `region` or overhang constraint. Give them
  more room: a larger `outline` (`room.suggested_outline_mm`), a wider
  `region`, a looser `edge` `max_mm`, parts on the other side, or unfix a
  neighbour.
  `placed/board-placer.json` has the same `hints` and the `utilization`
  per side.
  Before giving up, the placer shrinks the halos and the spacing (down to
  the copper clearance) and retries legalization with the stuck parts
  first. As a last resort it lets courtyards overlap: bodies shrink to
  their fabrication outline and pads (`"tight_bodies": true` in
  `board-placer.json`). It does that only where the project's DRC does not
  treat a courtyard overlap as an error; `"tight_bodies": false` in the
  placer config forbids it. Parts held at an edge then need only their
  copper on the board (`"edge_copper": true`): a card edge's fingers fill
  a tab of the outline that their courtyard overhangs, key notch and all.
- **A `near` or `relative` constraint is not `satisfied`.** Geometry forbids
  it, or it conflicts with another constraint; `violation_mm` says by how
  much. Relax the distance, or fix the part yourself.
- **Connections stay open.** Read `routed.diagnostics`. `hot_spots` say
  where to make room (move parts apart, enlarge the outline, add layers);
  `dead_pads` name pads whose class is too wide for them. Otherwise the
  board is too dense for its layer count or size. Enlarge the outline or add layers. `benchmarks/corpus/run.py
  --shrink` shows how much a design can shrink.
- **Pads or copper too close to the edge.** A `flush` part's courtyard
  touches the edge; its pads still need the board's copper-to-edge
  clearance, which DRC reports.

## 5. Order it

```sh
pcb-maker export-kicad-fab <out-dir>/result <board-id> <fab-dir>
```

writes what a fab takes: `<board-id>-gerbers.zip` (copper, mask, paste,
silkscreen, outline, drill files; zones filled as KiCad fills them),
`<board-id>-bom.csv` and `<board-id>-cpl.csv` in JLCPCB's assembly format,
and `fab-export.json` with the estimated price at common fabs. BOM lines
take the LCSC part number from a footprint property (`LCSC`, `LCSC Part`,
`JLCPCB Part #`); parts without one are listed in `without_lcsc`. Check the
rotations of polarised parts in the fab's preview: library orientations
differ between KiCad and assemblers.

## Check a board with KiCad

```sh
pcb-maker drc-kicad-board <project-dir> <board-id> <report.json> [--no-refill]
```

runs KiCad's DRC on the board and its rules (`.kicad_pro`, `.kicad_dru`;
no schematic parity), zones refilled unless `--no-refill`, through the DRC
cache (section 4), and writes KiCad's report (every severity) to
`<report.json>`. One line on stdout counts errors by type, unconnected
items, warnings and cosmetic findings, and the cache's calls and hits.
Exit 0 when no error counts (warnings, excluded and cosmetic findings do
not) and no item is unconnected, 1 otherwise, 2 for a missing project or
board, 4 when KiCad fails. The benchmark runners (`run.py`'s designer
check, `freerouting.py`) use it when the binary has it.

## Score a board

```sh
pcb-maker score-kicad-board <project-dir> <board-id>
```

reports objective measures ([quality.md](quality.md)); `layout-kicad-board`
puts the same report into `board-layout.json` as `quality`. It covers:
- decoupling capacitor distance to every IC supply pin, and each
  capacitor's distance to its nearest supply pin;
- crystal path lengths; switching inductors (a regulator's) near
  crystals, antenna modules and analog parts (op-amps, ADCs, codecs,
  references);
- how much other copper cuts the ground pours (return paths);
- hot parts (a regulator's tab, an exposed pad): the pad's area, the
  planes of its net over it, vias in it and track copper near it. A pour
  of that net (`add_pours`) is the cheapest heat sink;
- orientation consistency, parts near the edge, connectors facing out;
- vias in pads, tombstoning risk, KiCad DRC counts;
- the estimated price of 10 boards at common fabs, and the price
  thresholds the board crosses ([cost.md](cost.md)).

Score the designer's board and yours side by side.

## Quality you can expect

Measured results are in [benchmarks.md](benchmarks.md) (corpus) and
[../benchmarks/pcbench/README.md](../benchmarks/pcbench/README.md)
(PCBench/D3, about 630 open-source boards).

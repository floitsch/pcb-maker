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
place of the schematic. All footprints sit at one point and there is no
outline. Next to the board it writes, unless they exist:
- `constraints.json`: `"move_all": true`, `"outline": {}` (sized from the
  parts), and the plug-in connectors (USB, RF, audio and barrel jacks,
  D-sub, network, video, card edges, terminal blocks) on `"edge": "any"`,
  facing out. Edit it: it is where what the user wants goes.
- `layout.json`, which uses it, so that
  `pcb-maker layout-kicad-board <project-dir> <board-id> <out-dir> auto <project-dir>/layout.json`
  lays the board out.

The report lists what needs a look:
- `missing`: footprints not found in any library (the board leaves them
  out), or unreadable;
- `substituted`: footprints found in another library or under KiCad 9's
  name (`DSUB-25_Female` is now `DSUB-25_Socket`);
- `unmatched_pins`: symbol pins on a net that the footprint has no pad
  for (a library that changed since the schematic was drawn);
- `clearances`: footprints whose own pads are closer than the board's
  clearance (a solder jumper's), given their pads' gap as their own
  clearance, as designers do.

## 1. Look at the board

```sh
pcb-maker describe-kicad-board <project-dir> <board-id> > board.json
```

The file lists what constraints refer to:
- the outline's box and the copper layers;
- every footprint with reference, value, library name, position, side,
  size, whether it is through-hole or locked, and its pads with their
  nets;
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
- **`outline`** sets the board size.
- **`edge`, `region`, `keepout`, `rotation` and `fixed`** decide where parts may be;
  **`back`/`front`** on which side.
- **`hollow`** lets parts sit inside a big part's outline (a shield, a
  module above parts): only its pads block. Parts with holes still stay
  out of its courtyard's shapes, as KiCad forbids holes inside a
  courtyard.
- **`near`, `group` and `relative`** keep parts together; **`apart`** keeps
  them away from each other; **`row`** puts parts in a line at a pitch (LED
  bars, key rows).
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
| "The connector must match the enclosure cut-out" | `"place": [{"part": "J3", "x": 12, "y": 0, "angle": 90}]` |
| "Status LED in the top-right corner" | `"place": [{"part": "D1", "at": "top-right"}]` |
| "Eight LEDs in a row, 5 mm apart, along the top" | `"row": [{"parts": ["D1", "D2", ..., "D8"], "pitch_mm": 5}]`, `"place": [{"part": "D1", "at": "top"}]` |
| "A 4 x 4 key matrix, 19.05 mm pitch" | `"row": [{"parts": ["SW*"], "pitch_mm": 19.05, "columns": 4}]` |
| "WiFi antenna over the board edge" | `"edge": [{"part": "U1", "edge": "top", "flush": true, "overhang": {"edge": "top"}}]` |
| "Decoupling caps at their IC's supply pins" | `"near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]` (`REF:PAD`, the pad number; `describe-kicad-board` lists each net's pads) |
| "Keep the temperature sensor away from the regulator" | `"apart": [{"parts": ["U4"], "from": ["U2"], "min_mm": 10}]` |
| "Keep the power supply together, bottom right" | `"group": [{"parts": ["U2", "L1", "C1?"], "max_mm": 4}]`, `"place": [{"part": "U2", "at": "bottom-right"}]` |
| "Nothing under the display window" | `"keepout": [{"x": [10, 40], "y": [5, 25]}]` |
| "No copper under the antenna" | `"keepout": [{"x": [...], "y": [...], "copper": true}]` |
| "Battery holder on the back" | `"back": ["BT1"]` |
| "A HAT: the Pi's outline is fixed, parts go inside it" | `"fixed": ["J5"]`, `"hollow": ["J5"]` |
| "As small as possible" | `"outline": {"shrink": true}`: the smallest board a quick route completes (each size tried costs a placement and a route) |

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
     `pcb-maker place-kicad-board <dir> <id> <out> placer.json`, where
     `placer.json` is the `placer` object of `layout.json`.
     `<out>/board-placer.json` reports every constraint, `unplaced`,
     `hints` and `utilization`.
  2. Make a draft: `{"moves": 0, "placer": {...}}` in `layout.json` places
     and routes once, skipping the router-driven part moves. Those moves
     take most of a layout's time.
  3. Make the final run with the defaults. It makes up to 40 moves, for
     at most 600 s while connections are open and 60 s
     (`polish_seconds`) once everything is routed.

## 4. Read the verdict: `<out-dir>/board-layout.json`

| Field | Meaning |
| --- | --- |
| `routed.routed_connections` / `routable_connections` | Connections made / needed. |
| `routed.unconnected_terminals` | Pads the router could not reach. |
| `routed.native` | KiCad's own verdict: `complete`, `drc_design_violations`, `selected_net_unconnected_items`, `schematic_parity_issues`. Silkscreen findings show up here too; `repair-kicad-silkscreen` fixes most of them. |
| `routed.vias`, `routed.length_mm` | Cost of the routing. |
| `constraints[]` | Every constraint: `kind`, `part`, `other`, `satisfied`, `violation_mm`. |
| `routed.diagnostics.dead_pads[]` | Pads no track of their net's class can enter (`pad`, `net`, `at`, the objects `near` it): a class too wide for the pad pitch, or something covering the pad. |
| `routed.diagnostics.conflicted_nets` | Nets still fighting for room when the router gave up. |
| `routed.diagnostics.hot_spots[]` | Where they fought longest (`layer`, `at`, `history`), worst first: give these areas more room. |
| `constraint_warnings` (in `placed/board-placer.json`) | Inputs accepted but not acted on. |

## When it does not work out

- **`placement is not legal (unplaced [...]); hints`.** No position
  satisfies the hard constraints for these parts. The hints after the
  semicolon say why: a side that is more than 70 % full (parts' bodies
  with spacing against the board area), or parts held by an `edge`,
  `region` or overhang constraint. Give them
  more room: a larger `outline`, a wider `region`, a looser `edge`
  `max_mm`, parts on the other side, or unfix a neighbour.
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

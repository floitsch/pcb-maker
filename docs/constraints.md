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
 "fixed":    ["J1", "H*"],
 "rotation": [{"part": "U1", "angle": 90}, {"part": "D1", "angles": [0, 180]}],
 "edge":     [{"part": "J4", "edge": "left", "flush": true},
              {"part": "U3", "edge": "left", "flush": true, "overhang": {"edge": "top"}}],
 "region":   [{"parts": ["U5", "C1?"], "x": [0, 20], "y": [0, 15]}],
 "near":     [{"part": "C1", "pin_of": "U1:48", "max_mm": 3},
              {"part": "U2", "part_of": "J4", "max_mm": 10}],
 "relative": [{"part": "J4", "below": "U3", "max_gap_mm": 3}]}
```

Parts are named by reference. `fixed` and `region.parts` also take glob patterns
(`*`, `?`). Coordinates are millimetres, in KiCad's orientation: y grows
downwards, so `top` is the smaller y.

| Key | Meaning | Kind |
| --- | --- | --- |
| `outline` | The board becomes a `width` × `height` rectangle, replacing the board's Edge.Cuts. `x`/`y` place its top-left corner (default: the old outline's corner; without one, centred on the footprints). Parts lying entirely off the new board are placed, not kept. | board |
| `fixed` | These parts keep the pose they have in the board file. | hard |
| `rotation` | The part may only take these KiCad orientations (degrees). | hard |
| `edge` | The part's body lies within `max_mm` (default 1; 0 with `flush`) of that edge of the outline's bounding box. On that side the courtyard may touch the edge: copper clearance to the edge is still enforced by the router and DRC. | hard |
| `edge.overhang` | The part reaches beyond `overhang.edge` with its footprint's keepout zone (an ESP32 module's antenna). The keepout lies outside the board, flush with the edge; the rest of the body inside. Only orientations that point the keepout at that edge are allowed. The footprint must have a keepout zone. | hard |
| `region` | The bodies stay inside the box. By default `x`/`y` are relative to the top-left corner of the outline's bounding box; `"origin": "absolute"` uses board coordinates. | hard |
| `near` | The gap between the part's body and another part's body (`part_of`) or a pad (`pin_of`, `REF:PAD`) is at most `max_mm`. | soft |
| `relative` | The part lies `below`/`above`/`left_of`/`right_of` another part: it does not overlap it along that axis, its centre lies within the other part's extent across it, and the gap is at most `max_gap_mm` (default 5). | soft |

A part named by any constraint may move even where a default rule would fix
it (a connector at the edge, a part hanging over the outline), unless `fixed`
names it.

**Hard** constraints are part of legality. Every placement stage keeps them:
global placement, annealing, legalization, refinement and the router-driven
nudges. A board where no legal position exists reports the part as `unplaced`.

**Soft** constraints are a penalty: one millimetre of violation costs as
much as `constraint_weight` (default 10) millimetres of wire. They act as a
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
  {"kind": "near", "part": "C5", "other": "U3", "satisfied": false, "violation_mm": 3.235}
]
```

`constraint_warnings` lists accepted but unused inputs. An unknown key, part
or pad is an error, never ignored.

## Not supported yet

- `opening_outwards` for connectors: accepted with a warning; give a
  `rotation` instead.
- Putting a part on the other side of the board (side assignment).
- Keepout regions for placement other than the board's own rule areas.
- Groups ("keep these together") other than pairwise `near`.

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

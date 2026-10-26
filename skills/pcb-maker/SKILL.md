---
name: pcb-maker
description: Place, route, check and release KiCad boards with the pcb-maker CLI. Use when asked to lay out or route a KiCad PCB from a schematic or netlist, to route around hand-routed (pre-routed) traces, to check a board with KiCad's DRC, or to produce fab files (Gerbers, drill, BOM, placement) gated by DRC.
---

# pcb-maker

pcb-maker places and routes KiCad boards and lets KiCad's own DRC judge
the result. It needs `kicad-cli` (KiCad 10) on the PATH, or
`PCB_KICAD_CLI` naming it: route, layout, DRC, release and a schematic's
import check it first and exit 2 without it. Every command
prints its usage with `pcb-maker <command> --help`; the full guide is
docs/agents.md in the pcb-maker repository.

## Workflow

1. From a schematic: `pcb-maker import-kicad-netlist <dir>/<name>.kicad_sch <project-dir> <id>`
   writes the board, `constraints.json` and `layout.json`.
2. Look at the board: `pcb-maker describe-kicad-board <project-dir> <id>`
   (parts, pads, nets, `REF:PAD` names for constraints).
3. Say what the user wants in `<project-dir>/constraints.json` (below).
4. Check the constraints in seconds:
   `pcb-maker place-kicad-board <project-dir> <id> <out> <project-dir>/layout.json`.
5. Lay out: `pcb-maker layout-kicad-board <project-dir> <id> <out> auto <project-dir>/layout.json`;
   the finished project is `<out>/result`.
   A board already placed: `pcb-maker route-kicad-board <project-dir> <id> <out> auto`.
6. Release: `pcb-maker fab-kicad-board <out>/result <id> <release-dir> [--profile jlcpcb]`.

Nets left open: `pcb-maker inspect-routing-run <out> --reference <project-dir>` names each
open net's groups of pads (ref, number, uuid, position, layers), KiCad's pairs, the rules and
their sources, the designer's route, the hot spots' pads and nets and the ladder's attempts;
`access` and `cause` stay `unknown`. Read it before any design change: where the designer's
board routes the open nets (`reference.connected: true`), density is not the reason.

Other commands: `drc-kicad-board` (KiCad's DRC, cached), `score-kicad-board`
(quality measures and price), `repair-kicad-silkscreen`, `swap-kicad-pins`,
`export-kicad-fab` (fab files without the DRC gate), `strip-kicad-tracks`,
`strip-kicad-copper`. Output directories must not exist yet.

## Read the verdict

Every run writes `<out>/verdict.json`; the exit code says the same.

| Exit | `status` | Do |
| --- | --- | --- |
| 0 | `complete` | Use the result. |
| 1 | `incomplete` (or `router_complete_unverified`) | Read `next_actions` and act on them; for a release, fix the findings. |
| 2 | `usage_error` | Fix the argument or input file the message names; nothing ran. |
| 3 | `cut` | The wall clock cut the run: rerun with a larger budget or on an idle machine. |
| 4 | `internal_error` | Report the error with the input. |

Fields to read: `reason` (one line), `next_actions` (what to do, in
order), `kicad_unconnected_items` and `new_findings_strict` (they decide),
`constraints_missed`, `unplaced`, `room` (`suggested_outline_mm`: an
outline that fits), `outputs` (absolute paths), `cut`, `free_dimension`,
`fab_release`.

On exit 1: for open connections run `inspect-routing-run` first (above);
too little room is a hypothesis, not a finding (`routed.diagnostics.hot_spots`
in `board-layout.json` says where nets fought, not why), so enlarge the
outline, move parts apart or add layers only where the reference does not
route the same nets; unplaced parts mean a fuller board than fits
(`room.suggested_outline_mm`); missed constraints give `violation_mm`:
relax or fix the part.

## constraints.json

`{"version": 1, "move_all": true, "outline": {"width": 60, "height": 40}, ...}`.
Keys: `place` (`"at": "top-left"`, or `x`, `y`, `angle`), `edge` (a part
flush on an edge, `opening_outwards`), `near` (`pin_of: "U1:3"`,
`max_mm`), `group`, `relative`, `apart`, `row` (parts at a pitch),
`region`, `keepout` (`copper: true` for no copper), `back`/`front`,
`fixed`, `rotation`, `hollow`, `overhang`, `add` (parts the schematic
lacks), `outline` (`width`/`height`, `corner_radius`, `polygon`,
`cutouts`, `pieces`, `shrink`). Unknown keys and parts are errors (exit 2).

## layout.json

`{"placer": {"constraints": "constraints.json"}, "total_seconds": 1500}`.
`total_seconds` is the budget in seconds of work (30 lays out a small
keyboard); `moves: 0` makes a quick draft; `swappable` lets interchangeable
pins be reassigned; `free_dimension` (`{"edge": "bottom", "min": 34,
"max": 60, "step": 2}`) lays the board out at several sizes of one edge
and keeps the smallest clean one (give `total_seconds` several layouts'
worth).

Router options go in a JSON file in place of `auto`: `add_pours`
(`[{"net": "GND", "layers": ["B.Cu"], "plane": true}]`), `net_classes`
(`track_width_mm`, `clearance_mm` per net), `seeds`, `via_in_pad`.

## Pre-routed nets

Route critical traces by hand (or through KiCad's API), then
`route-kicad-board`: the board's own tracks and vias are kept unchanged
(never ripped up), pads they join count as connected, and only what is open is routed
(`kept_copper` in `board-router.json`). `--strip` routes from nothing.
A layout always strips (it moves parts).

## Fab release

`fab-kicad-board <project-dir> <id> <release-dir>` runs KiCad's DRC (and
ERC with a schematic) and refuses (exit 1, the findings in `next_actions`)
while copper errors or unconnected items remain; cosmetic findings do not
block, `--allow-findings` overrides. Released (exit 0): `gerbers/` and
`<id>-gerbers.zip` (upload this), `assembly/` (position file, BOM),
`netlist/`, `<id>.step`, and `fab-manifest.json` (every file's sha256, the
DRC and ERC counts, KiCad's version). `--profile jlcpcb` adds JLCPCB's BOM
and placement columns; `--layers` picks the Gerber layers.

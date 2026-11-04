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
4. Check the constraints: the layout's placement alone (its budget, seeds and
   router rules, at the reference's work rates: the layout's placement at the same
   rates; a no where parts cannot fit in seconds):
   `pcb-maker place-kicad-board <project-dir> <id> <out> <project-dir>/layout.json`.
   A panel of different boards (pieces joined by tabs, mouse bites or V-cuts) whose
   nets do not tell the boards apart needs `--reference <designer's project>` here and
   in the layout (`verdict.sub_boards`).
5. Lay out: `pcb-maker layout-kicad-board <project-dir> <id> <out> auto <project-dir>/layout.json`;
   the finished project is `<out>/result`.
   A board already placed: `pcb-maker route-kicad-board <project-dir> <id> <out> auto`.
6. Release: `pcb-maker fab-kicad-board <out>/result <id> <release-dir> [--profile jlcpcb]`.

Nets left open: `pcb-maker inspect-routing-run <out> --reference <project-dir>` names each
open net's groups of pads (ref, number, uuid, position, layers), KiCad's pairs, the rules and
their sources, the designer's route, the hot spots' pads and nets and the ladder's attempts;
`access` and `cause` stay `unknown`. Read it before any design change: where the designer's
board routes the open nets (`reference.connected: true`), density is not the reason.

Checking a result with `kicad-cli pcb drc` directly: add `--refill-zones` (the written
zones are unfilled; without it pour pads count as unconnected).

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
order: this run's actions first; the input's own observations last, each
starting with `nothing to do (the input's): `, those of one kind folded
into one line, each in full in `input_observations`; fab-kicad-board's and
drc-kicad-board's `next:` lines in the same order), `kicad_unconnected_items` and `new_findings_strict` (they decide),
`constraints_missed`, `unplaced`, `room` (`suggested_outline_mm`: an
outline that fits), `outputs` (absolute paths), `cut`, `free_dimension`,
`fab_release`; `custom_rules` (each `.kicad_dru` constraint: `applied`,
`approximated`, `no_effect`, `unmodelled`, ...), `lengths` (the `length`
and `skew` rules per net), `sub_boards`, `ledger_wait_seconds` (the part of
`wall_seconds` spent waiting in the machine's memory ledger, not working). Without a reference (a layout's
default) a footprint's own findings (`footprint_findings`) and the
input's own edge clearances, mask bridges and courtyards of parts kept
where the input has them (`input_findings`) do not count; `next_actions`
gives the custom rule or the exclusion that makes KiCad accept them.
Clearances and shorts between fixed parts always count.

On exit 1: where the first next action says the run searched at under half
the reference's rate and the first route took the whole budget, rerun as it
says (idle machine or the pinned rates) before anything else (SmartSpin2k: 475
unconnected so, 51 pinned). The hot spot's action names its open link's far
end with a `near` entry to paste. For open connections run `inspect-routing-run` first (above;
with `--reference` its `reference.placement` says whether the designer's
route is on this placement or, for a layout, on the designer's own, with
the layout's own routes as `evidence_for_this_placement`);
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
On a panel of copies (`verdict.panel_copies`) a `near`, `relative` or `apart` between two
parts of one copy holds in every copy (write it once); an `edge`, `region`, `place` or
`rotation` entry holds in the copy whose footprint it names (docs/agents.md, "Panels of copies").

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
(`track_width_mm`, `clearance_mm` per net), `seeds`, `via_in_pad` (by default a
filled and capped via goes only into a BGA's enclosed pads where the designer's board,
the input or `--reference`, has vias in that footprint's pads; `["U106"]` names more
parts; the verdict names the part when such pads stay open), `diff_pairs` (route mode couples
the pairs where the project's `.kicad_dru` asks for them; a layout only with `"diff_pairs": true`;
a pair couples between two parts whose pads of the pair lie side by side, so keep a pair's
AC-coupling or series parts in a `row` of two (or the placer's `"pair_rows": true`, off by
default, which rows them itself at the pair's pitch, facing the pair's ends); `verdict.diff_pairs`
says which coupled and why the others fell back; a layout couples them only in its final
ladder: where `verdict.diff_pairs` is null, route the layout's `placed` board in route mode).

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
block, nor errors within one footprint each (a library footprint's pads at
its own holes: `footprint_note` gives the custom rule for the board's
`.kicad_dru` that makes KiCad accept them), `--allow-findings` overrides. Released (exit 0): `gerbers/` and
`<id>-gerbers.zip` (upload this), `assembly/` (position file, BOM),
`netlist/`, `<id>.step`, and `fab-manifest.json` (every file's sha256, the
DRC and ERC counts, KiCad's version). `--profile jlcpcb` adds JLCPCB's BOM
and placement columns; `--layers` picks the Gerber layers. The board is judged on its
own (every part and zone the board's); `--input <project-dir>` (the project it was laid
out from) judges it as the layout's verdict did, and names the findings between parts
kept where the input has them. `drc-kicad-board` takes `--input` too.

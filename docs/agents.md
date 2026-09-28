# pcb-maker for agents

You have a KiCad project: a `.kicad_pcb` with footprints and nets, from
"Update PCB from Schematic", kinet2pcb, SKiDL or a generator; a schematic is
optional. pcb-maker places the parts, routes the board, and tells you
precisely what it achieved.

## 1. Say what you want: `constraints.json`

```json
{"version": 1,
 "outline": {"width": 60, "height": 40},
 "edge": [{"part": "J1", "edge": "left", "flush": true}],
 "near": [{"part": "C1", "pin_of": "U1:3", "max_mm": 2}]}
```

Full reference: [constraints.md](constraints.md).
- **`outline`** sets the board size.
- **`edge`, `region`, `rotation` and `fixed`** decide where parts may be.
- **`near` and `relative`** keep parts together.
- **`overhang`** lets a module's antenna reach beyond an edge.

Unknown keys, parts or pads are errors, not silently ignored.

## 2. Run

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
  - `pin_swaps`: see below.
- **Pin swapping.** `"pin_swaps": "pin-swaps.json"` lets pcb-maker reassign
  interchangeable pins (GPIOs, resistor-network elements). The schematic is
  rewritten to match ([pin-swap.md](pin-swap.md)).
- **Output.** `<out-dir>/result/` holds the finished project, copied, with
  the placement and the copper. `<out-dir>/placed/placement.html` animates
  the placement.

## 3. Read the verdict: `<out-dir>/board-layout.json`

| Field | Meaning |
| --- | --- |
| `routed.routed_connections` / `routable_connections` | Connections made / needed. |
| `routed.unconnected_terminals` | Pads the router could not reach. |
| `routed.native` | KiCad's own verdict: `complete`, `drc_design_violations`, `selected_net_unconnected_items`, `schematic_parity_issues`. Silkscreen findings show up here too; `repair-kicad-silkscreen` fixes most of them. |
| `routed.vias`, `routed.length_mm` | Cost of the routing. |
| `constraints[]` | Every constraint: `kind`, `part`, `other`, `satisfied`, `violation_mm`. |
| `constraint_warnings` (in `placed/board-placer.json`) | Inputs accepted but not acted on. |

## When it does not work out

- **`placement is not legal (unplaced [...])`.** No position satisfies the
  hard constraints for these parts. Give them more room: a larger
  `outline`, a wider `region`, a looser `edge` `max_mm`, or unfix a
  neighbour.
- **A `near` or `relative` constraint is not `satisfied`.** Geometry forbids
  it, or it conflicts with another constraint; `violation_mm` says by how
  much. Relax the distance, or fix the part yourself.
- **Connections stay open.** The board is too dense for its layer count or
  size. Enlarge the outline or add layers. `benchmarks/corpus/run.py
  --shrink` shows how much a design can shrink.
- **Pads or copper too close to the edge.** A `flush` part's courtyard
  touches the edge; its pads still need the board's copper-to-edge
  clearance, which DRC reports.

## Quality you can expect

Measured results are in [benchmarks.md](benchmarks.md) (corpus) and
[../benchmarks/pcbench/README.md](../benchmarks/pcbench/README.md)
(PCBench/D3, about 630 open-source boards).

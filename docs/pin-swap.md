# Pin swapping

Many pins are interchangeable: any GPIO of a microcontroller can drive a
given signal, any element of a resistor network can serve any channel, any
X port of a crosspoint switch any row. Choosing *which* pin carries which net
changes how hard a board is to route. pcb-maker can make that choice, in the
board and in the schematic, and report it (firmware usually takes its pin
tables from it).

```sh
# Swap only: writes a copy of the project with the new assignment and
# pin-swaps-result.json (every changed pad, before and after).
pcb-maker swap-kicad-pins <source-directory> <board-id> <output-directory> <pin-swaps.json> [config.json]
```

- In **route mode**, `{"pin_swaps": "pin-swaps.json"}` in the router config
  swaps before routing.
- In **place + route mode**, the same key in the layout config swaps after
  placement.
- Paths are relative to the source directory. The chosen assignment is
  recorded in `board-router.json` / `board-layout.json` under `pin_swaps`.

Code: `crates/pcb-kicad/src/pin_swap.rs`.

## Input: `pin-swaps.json`

This is the format the breadboard fence generator writes
([benchmarks/fence/README.md](../benchmarks/fence/README.md#proposed-input-formats)):

```json
{"version": 1,
 "components": {
   "U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"], ["3"]],
                      "restrictions": {"MCU_PROBE": ["10", "11"]}}]},
   "*resistor-networks": {"groups": [{"name": "series resistors", "cross_component": true,
                                      "units": [["RN1:1", "RN1:8"], ["RN1:2", "RN1:7"]]}]}}}
```

- **Group.** The nets on its units may be permuted freely.
- **Unit.** A unit's pins move together. The unit keeps its orientation:
  the net on the first pin stays on the first pin.
- **`restrictions`.** Limits a net (named without its sheet path) to units
  that contain one of the listed pins.
- **`cross_component`.** The group's units belong to several footprints,
  and pins are written `REF:pad`. A key starting with `*` implies it.

## How the assignment is chosen

The assignment is chosen by simulated annealing, finished with a steepest
descent over all exchanges, on a ratsnest estimate of the board:

- **Nets and pads.** Each net is a minimum spanning tree over its pads.
  Nets with more than `maximum_net_pads` pads (default 12) are left out:
  they are ground and supply, and are poured or routed wide.
- **Escapes.** A pad of a surface-mount part with four or more pads is
  taken to leave the part outwards: its tree starts at an *escape point*
  1 mm outside the pad field, on the side where the pad lies. A pin on the
  far side of a chip therefore cannot pretend to reach straight across it.
- **Cost.**
  - The total length.
  - Plus `crossing_cost_mm` (default 3) for every crossing of two trees of
    different nets.
  - Plus `body_crossing_cost_mm` (default 10) for every tree edge that runs
    through a pad field.

The random generator is seeded (`seed`), so a run is repeatable.
`moves_per_unit` (default 2000) sets the annealing length. The two fence
boards (135 and 147 units) take 8-10 s.

## Output: board and schematic

- **Board.** The pads get their new nets. A pad whose unit ends up without
  a net gets the name KiCad gives an unconnected pin,
  `unconnected-(REF-PINNAME-PadN)`. The pin name is taken from the
  schematic symbol.
- **Schematic** (single sheet). Every changed pin is handled by how it is
  attached:
  - **Wire stubs that end in labels** (what generators write). The labels
    are renamed.
  - **Wires that reach other pins.** The unbranched wire run from the pin to
    the first label, junction, branch or pin is removed. If the rest of the
    net has no label of its name left, it gets one at the cut. The pin
    gets a label of its new net.
  - **A pin that loses its net.** Its stub and labels go, and it gets a
    no-connect flag.
  - **A pin that gains a net.** It gets a label; a no-connect flag on it is
    removed.
- **Refused.**
  - Nets that KiCad names after a pin (`Net-(...)`). Their name would
    change with the schematic.
  - Pins in hierarchical sub-sheets.

On both fence boards, KiCad reports no schematic-parity issue and no ERC
finding after the swap.

## Results on the fence boards (2026-09-25)

On the 2-layer v4 fence boards (`benchmarks/fence/v4-2layer`), the hand-made
assignment and the swapped ones were routed side by side, under the same
machine load, with `route-kicad-board ... auto`:

| Board | Assignment | Ratsnest estimate (length, crossings, lines through pad fields) | Routed | Unconnected pads | Vias | Copper |
| --- | --- | --- | --- | --- | --- | --- |
| fence | hand-made (generator) | 2436 mm, 897, 82 | **91/101** | 14 | 218 | 3066 mm |
| fence | swapped, plain ratsnest | 2263 mm, 635 (plain estimate; hand-made: 2288 mm, 717) | 87/101 | 24 | 223 | 2873 mm |
| fence | swapped, escape-aware | 2410 mm, 746, 62 | 85/101 | 23 | 176 | 2719 mm |
| fence-esp32 | hand-made | 2533 mm, 909, 114 | 82/108 | 58 | 162 | 2323 mm |
| fence-esp32 | swapped, plain ratsnest | 2351 mm, 603 (plain estimate; hand-made: 2366 mm, 704) | 81/108 | 60 | 190 | 2423 mm |
| fence-esp32 | swapped, escape-aware | 2530 mm, 751, 92 | 82/108 | **44** | 215 | 2566 mm |

Pour nets as tracks only, fence, one attempt each:
- hand-made: 91/101, 218 vias;
- U1's GPIOs left alone: 86/101, 197 vias;
- crossings at 10 mm and pad fields at 30 mm: 88/101, 209 vias.

**Reading.** The swaps do what the estimate asks for: 15-30 % fewer
crossings, and fewer vias or less copper in most runs. But they do not
finish more nets than the hand-made assignment. Two reasons:
- The generator's assignment was already chosen to be crossing-free
  between the MCU and the networks (0 crossings among the `MCU_ROW`
  nets).
- On these boards the router's result is set by the negotiation stall at
  U1's escape ring ([failure-analysis.md, F4](failure-analysis.md#f4-negotiation-stalls-in-saturated-escape-regions)).
  Which nets end up stuck there changes with any change of the input, and
  a straight-line estimate cannot see it.

The feature therefore stays opt-in. Its value on boards like these depends
on F4, or on judging assignments by the router (next steps).

## Next steps

- **A better judge than the ratsnest.** The estimate decides by straight
  lines. The router's tile graph (corridor planning with capacities) could
  score an assignment by congestion instead, which is what actually fails
  on these boards ([failure-analysis.md](failure-analysis.md#f4-negotiation-stalls-in-saturated-escape-regions)).
- **Swapping in the loop.** Swapping in the coupled layout loop:
  re-assign after router-driven moves, or let the router propose exchanges
  of pins whose nets conflict.
- **Reversible units.** A resistor element works either way round; the
  format could say so.
- **Hierarchical schematics.**

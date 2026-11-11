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
- In **place + route mode**, the same key (or `swappable`, below) in the
  layout config swaps after placement. A layout with pin swaps does not
  pre-rank or race the placer's seeds.
- Paths are relative to the source directory. The chosen assignment is
  recorded in `board-router.json` / `board-layout.json` under `pin_swaps`.

Code: `crates/pcb-kicad-route/src/pin_swap.rs`. Pin swapping is opt-in: on the
breadboard fence boards it cut the ratsnest's crossings by 15 to 30 % but
did not finish more nets, because which nets stall at the MCU's escape ring
changes with any input and a straight-line estimate cannot see it.

## The short way: `swappable` in the layout config

```json
{"placer": {...},
 "swappable": [{"part": "U1", "pins": ["GPIO*", "IO*"], "except": ["GPIO0", "GPIO2"]},
               {"part": "J3", "pins": ["3", "4", "5", "6"]}]}
```

Per part, the pins whose nets may be permuted: glob patterns over pad
functions (`pinfunction` in the board, from a KiCad 6+ schematic) or pad
numbers; `except` leaves some out (strapping pins, a bootloader's UART).
Each matching pad is a unit of one group. It is used when `pin_swaps` is
not given.

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
  the net on the first pin stays on the first pin. (A resistor element
  that works either way round cannot be marked reversible.)
- **`restrictions`.** Limits a net (named without its sheet path) to units
  that contain one of the listed pins.
- **`cross_component`.** The group's units belong to several footprints,
  and pins are written `REF:pad`. A key starting with `*` implies it.

## How the assignment is chosen

The assignment is chosen by simulated annealing, finished with a steepest
descent over all exchanges, on a ratsnest estimate of the board. The
settings are the `pin_swap` object of the router or layout config (or the
`config.json` of `swap-kicad-pins`):

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

The random generator is seeded (`seed`, default 0), so a run is
repeatable. `moves_per_unit` (default 2000) sets the annealing length; a
board of about 140 units takes 8 to 10 s.

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

After a swap of the fence boards, KiCad reports no schematic-parity issue
and no ERC finding.

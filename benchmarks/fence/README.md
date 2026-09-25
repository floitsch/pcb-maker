# Breadboard fence: benchmarks and findings

These boards come from a separate hardware project, the "breadboard fence" in
`~/programming/breadboard-enhancer/hw/` (generator `fence.toit`, written in
Toit; the finished boards are in its `kicad/`). That project uses pcb-maker as
its placer and router and never patches it. Instead, everything pcb-maker got
wrong, or could not express, is logged here together with the boards that
show it.

**For an agent working on pcb-maker:** the boards below are regression inputs
and feature targets.
- The `corpus` entries run with every `benchmarks/corpus/run.py` pass.
- `esp-usb-placement` and the `v1-*` boards run by hand (commands below).
- The issue log says which pcb-maker behaviour each one exposes and when it
  counts as fixed.
- Keep the boards as they are (they are frozen snapshots). Report results
  here, in the results table, and in [docs/benchmarks.md](../../docs/benchmarks.md).

## The board

A 76.2 × 31 mm board that lies flat over a breadboard's power rails. A 1×30
2.54 mm header on the **back** side (J1, THT) plugs into the 30 rows. Every
row has two paths:

- through one element of a 4×0603 resistor network (RN1–RN8, 1 kΩ) to a GPIO
  of the CH32X035 (U1, LQFP-48, 0.5 mm);
- directly to an X port of one of two CH446Q crosspoint switches (U4, U5,
  LQFP-44, 0.8 mm; pads 0.55 mm wide with 0.25 mm gaps, so no track fits
  between pads).

So every row net crosses between its network (towards the MCU) and its
switch: that crossing pattern is what makes the board hard. The two switches
share 8 bus nets (their Y ports) that go to a 1×8 header (J6) in the top row.

- **`fence`:** USB-C (J4) on the left edge, XC6206 regulator, 101 nets.
- **`fence-esp32`:** adds an ESP32-C3-MINI-1 (U3) in the top-left corner,
  flush with the left edge, its antenna overhanging the top edge; USB-C below
  it. 108 nets.
- **Stack (4-layer boards):** F.Cu signal, In1.Cu GND plane (`power`), In2.Cu
  and B.Cu signal.
- **Rules:** KiCad defaults (0.2 mm track and clearance, 0.6/0.3 mm vias)
  plus 0.3 mm copper-to-edge in the `.kicad_pro` (J1's end pads sit 0.42 mm
  from the side edges).

**Workarounds baked into the boards** (each hides an issue below):

| Workaround | In | Hides |
|---|---|---|
| A board-level rule area on In1.Cu named `GND plane: no tracks` | all 4-layer boards | issue 9 |
| The ESP32 footprint's antenna keepout repeated as a board-level rule area | every `fence-esp32` except `keepout-in-footprint` | issue 1 |
| Chips, decoupling, regulator, pump and networks fixed by hand in `layout.json` | all | issue 2 |
| A fixed row/pin assignment chosen by hand | all | issue 3 |

To test a fix, delete the rule area (or free the parts) in a copy.

## Benchmarks

Every directory is a KiCad 10 project (`<board_id>.kicad_pcb`, `.kicad_sch`,
`.kicad_pro`, project libraries) plus:
- `layout.json`: the `layout-kicad-board` config (fixed parts);
- `pin-swaps.json`: which pins are interchangeable (proposed format, see below).

| Directory | Board | State | What it measures | Pass |
|---|---|---|---|---|
| `v4-4layer/{fence,fence-esp32}` | current boards (hardware v4) | routed by pcb-maker (the reference result) | Route (designer placement) and place + route on 4 layers. | complete; no copper DRC error; no track on In1.Cu. Reference: 101/101 with 134 vias, 108/108 with 190 vias. |
| `v4-2layer/{fence,fence-esp32}` | v4 generated with `--layers=2` (GND pour on B.Cu instead of the plane), placement copied from `v4-4layer` | placed, unrouted | Can the same board be routed on 2 layers? | complete, with a GND pour that stays in one piece (or a plane-quality measure). Not reached yet (see results). |
| `keepout-in-footprint` | v1 ESP32 variant, 2 layers, **without** the board-level copy of the antenna keepout; placement from `v1-4layer` | placed, unrouted | The router honours a keepout zone defined inside a footprint. | no copper in the antenna area; KiCad reports no `items_not_allowed`. |
| `esp-usb-placement` | v2 ESP32 variant (4 layers, 34 mm) with U3, J4, U2, C3, C4, C5, C6 free and stacked | unplaced | Edge, overhang and relative placement. See [esp-usb-placement/README.md](esp-usb-placement/README.md). | its README's table. |
| `v1-4layer/{fence,fence-esp32}` | v1 (with a 2×29 socket strip threaded by the 30 row tracks) | routed by pcb-maker | Older, harder 4-layer variant. | complete. v1 result: 121 / 150 vias. |
| `v1-2layer/{fence,fence-esp32}` | v1 on 2 layers, placement from `v1-4layer` | placed, unrouted | The hardest variant. | complete. Not reached (92/100, 102/107). |

The corpus entries are named `fence-*` in
[`../corpus/corpus.json`](../corpus/corpus.json):

```sh
cargo build --release
python3 benchmarks/corpus/run.py build/corpus-fence --only fence-v4-4l fence-esp32-v4-4l \
    fence-v4-2l fence-esp32-v4-2l fence-keepout
```

The others, by hand (the `layout.json` next to each board is the config):

```sh
b=benchmarks/fence/esp-usb-placement
target/release/pcb-maker layout-kicad-board $b fence-esp32 build/fence-esp-usb auto $b/layout.json
b=benchmarks/fence/v1-2layer/fence
target/release/pcb-maker layout-kicad-board $b fence build/fence-v1-2l auto $b/layout.json
```

Check the result with native KiCad: `kicad-cli pcb drc --schematic-parity`
on the result; ERC on the schematic. Silkscreen findings don't count (issue 8).

## Results

| Date | Board | pcb-maker result (place + route with `layout.json` unless noted) |
|---|---|---|
| 2026-09-24 | v1 2 layers, `fence` | **incomplete:** 92/100, 259 vias. GND pour in 317 pieces, 10 starved thermals. Every ladder rung (plane, pours as tracks, 0.075 and 0.05 mm) left 18+ opens. |
| 2026-09-24 | v1 2 layers, `fence-esp32` | **incomplete:** 102/107, 325 vias. |
| 2026-09-24 | v1 4 layers | complete: 121 and 150 vias. |
| 2026-09-25 | v3 2 layers, `fence` (31 mm) | **incomplete:** 88/100, 185 vias. |
| 2026-09-25 | same, 4 / 8 / 12 mm taller (room between the chips and the networks) | 88/100 (234 vias), 97/100 (362 vias), **complete** at 12 mm (43 mm tall): 100/100, 273 vias, but the GND pour falls apart into 127 pieces and GND is carried by tracks; 1 starved thermal. |
| 2026-09-25 | v3 2 layers, `fence-esp32` (31 mm) | **incomplete:** 81/107, 198 vias. |
| 2026-09-25 | same, 4 / 8 / 12 mm taller | **incomplete:** 102/107 (310 vias), 103/107 (297 vias), 105/107 (356 vias). |
| 2026-09-25 | v3 4 layers | complete: 100/100 (130 vias), 107/107 (178 vias). |
| 2026-09-25 | v4 4 layers (`v4-4layer`) | complete: 101/101 (134 vias), 108/108 (190 vias). |
| 2026-09-25 | corpus route mode (designer placement, tracks stripped), `fence-v4-4l` | complete and clean: 101/101, 136 vias, 3214 mm, 171 s. |
| 2026-09-25 | corpus route mode, `fence-keepout` | **issue 1 reproduced:** 100/107, 324 vias; native KiCad: 9 unconnected, 11 `items_not_allowed` (copper in the footprint's antenna keepout), 9 starved thermals. |

| 2026-09-25 | v4 2 layers (`v4-2layer`), route mode, hand-made pins | **incomplete:** `fence` 91/101 (14 unconnected pads, 218 vias), `fence-esp32` 82/108 (58, 162 vias). Freerouting 2.2.4 on the cold `fence`: 8 unconnected items, 170 vias. Why: [docs/failure-analysis.md](../../docs/failure-analysis.md) F4/F5. |
| 2026-09-25 | same, pins swapped by pcb-maker | `fence` 85-88/101 with fewer vias (176-209) and less copper; `fence-esp32` 81-82/108 (44-60 unconnected pads). Not better than the hand-made assignment yet. |
| 2026-09-25 | `keepout-in-footprint`, route mode | **fails:** 96/107, 11 `items_not_allowed` (issue 1 confirmed; cause in docs/failure-analysis.md F3). |

(v3 and v4 differ only in the negative supply: a TPS60401 in v3, a diode
charge pump in v4. The v4 2-layer rows above are route mode with the designer's
placement.)

On 2 layers the opens are almost always row nets (`ROWn` / `MCU_ROWn`)
around chip A and the MCU: exactly where pin swapping (issue 3) would help
most. Whether a human could route the 31 mm board on 2 layers is open.

## Issue log

| # | Date | Severity | What happened | Status / workaround | Benchmark |
|---|---|---|---|---|---|
| 1 | 2026-09-24 | high | **Keepout zones inside footprints are ignored while routing.** The ESP32-C3-MINI-1 footprint carries its antenna keepout (tracks, vias, pads, pours not allowed) as a footprint zone. pcb-maker routed 13 tracks through it. Only KiCad's DRC caught it (`items_not_allowed`); pcb-maker reported 92/92 and exit 0. | Open. Worked around by repeating the keepout as a board-level rule area, which pcb-maker honours. | `keepout-in-footprint` |
| 2 | 2026-09-24 | high | **No placement constraints.** The placer only optimises wire length. Decoupling capacitors ended up far from their chips (the ESP32's C5/C6 on the far side of the board); the regulator drifted away from the USB connector and the module it feeds. `docs/placer.md`: "No constraint language yet (regions, groups, alignment, decoupling proximity, keep-near-edge)". | Open. Worked around by fixing those parts by hand. | `esp-usb-placement`; any `v4` board with its decoupling freed |
| 3 | 2026-09-24 | high | **No pin swapping.** Most of the fence's pins are interchangeable (see `pin-swaps.json`): any CH32X035 GPIO can serve a row, any resistor-network element any channel, any CH446Q X port any row of that chip, any ESP32-C3 GPIO the link lines. The fixed assignment is a hand-made guess at a crossing-free order. | Implemented 2026-09-25 (`swap-kicad-pins`, `pin_swaps` in the route/layout config; [docs/pin-swap.md](../../docs/pin-swap.md)); board and schematic stay parity-clean. On the v4 2-layer boards it does not yet finish more nets than the hand-made assignment (see results). | any board + its `pin-swaps.json`; compare vias, length and completion with the fixed assignment, and report the chosen mapping (the firmware takes its tables from it). |
| 4 | 2026-09-24 | medium | **Pad angles differ from KiCad's when a footprint's `(at …)` comes after its pads.** KiCad stores pad angles as absolute values and converts them with the rotation it has read so far. pcb-maker's model and KiCad's disagreed after rotating such footprints: 30 `shorting_items` inside resistor arrays, visible only in KiCad's DRC. | The generator now writes `(at)` first. pcb-maker could reject such files or read them like KiCad. | none kept |
| 5 | 2026-09-24 | low | **Overlapping fixed parts are not reported.** A too-broad `fixed_patterns` glob fixed six resistors still stacked at their initial position; the layout ended with 12 opens and no hint. | A warning ("fixed parts X and Y overlap") would save a round trip. | none kept |
| 6 | 2026-09-24 | low | **No snapshot release.** The README links `releases/tag/snapshot`, but it doesn't exist. | Built from the checkout. | – |
| 7 | 2026-09-24 | low | **Starved thermal relief** on a THT GND pad connected to a pour (1 of 4 spokes). | Cosmetic here. | `v1-2layer` |
| 8 | 2026-09-24 | cosmetic | **Silkscreen is not considered.** Reference designators overlap other silk and copper on every layout (`silk_overlap`, `silk_over_copper`: 56 and 94 findings on v4). | Open. | any |
| 9 | 2026-09-25 | medium | **Signals are routed on a plane layer.** In1.Cu is typed `power` and holds a full GND zone; pcb-maker still routed signal tracks on it, leaving large voids in the plane. The router config has no per-layer or per-net layer restrictions. | Worked around with the board-wide In1.Cu rule area. To test: delete it in a copy of a `v4-4layer` board. Pass: no tracks on In1.Cu, board still complete. | `v4-4layer` minus the rule area |
| 10 | 2026-09-25 | low | **Placement config:** `free` cannot release a part matched by `fixed_patterns`. A footprint that deliberately overhangs the outline (the ESP32's antenna) is treated as fixed by default (`on_edge`), and the placer has no notion of which part of a footprint may overhang. | `esp-usb-placement` lists its fixed parts explicitly and frees U3 and J4. | `esp-usb-placement` |
| 11 | 2026-09-25 | low | **Proximity again (issue 2):** the placer put the USB-C CC network (RN9) 25 mm from the connector's CC pins, and a charge pump's output capacitor 18 mm from the pump. | Hand-placed. | as issue 2 |

## Proposed input formats

### `pin-swaps.json` (written by the generator, next to every board)

```json
{"version": 1,
 "components": {
   "U1": {"groups": [{"name": "gpio", "units": [["1"], ["2"], "…"],
                      "restrictions": {"MCU_PROBE": ["10", "11", "…"]}}]},
   "*resistor-networks": {"groups": [{"name": "series resistors", "cross_component": true,
                                      "units": [["RN1:1", "RN1:8"], ["RN1:2", "RN1:7"], "…"]}]},
   "U4": {"groups": [{"name": "x", "units": [["31"], ["30"], "…"]},
                     {"name": "y", "units": [["33"], ["35"], "…"]}]}}}
```

- **Group:** within a group, the nets on its units may be permuted freely.
- **Unit:** its pins move together. A resistor-network element is one unit of
  two pins (`k` and `9−k`).
- **`restrictions`:** limits a net to a subset of the group's pins (the probe
  needs an ADC input).
- **`cross_component`:** units belong to different footprints, so a channel
  may move from one resistor network to another.
- **Output:** the tool reports the final pin→net mapping and writes a
  schematic that matches the board.

### `constraints.json` (placement intent; `esp-usb-placement` has a full example)

```json
{"version": 1,
 "edge": [{"part": "U3", "edge": "left", "flush": true,
           "overhang": {"edge": "top", "feature": "antenna keepout", "beyond_outline": true}}],
 "relative": [{"part": "J4", "below": "U3", "max_gap_mm": 3}],
 "near": [{"part": "C1", "pin_of": "U1:48", "max_mm": 3},
          {"part": "U2", "part_of": "J4", "max_mm": 10}]}
```

## Provenance

| Directory | Generated as (in `breadboard-enhancer/hw`) |
|---|---|
| `v4-4layer/*` | `kicad/fence`, `kicad/fence-esp32` on 2026-09-25 (hardware v4), routed by `layout-kicad-board` with `layout.json`; zones filled; J1 flipped to the back. |
| `v4-2layer/*` | `toit run fence.toit -- [--esp32] --layers=2 …`, J1 flipped (`finalize.py --flip-header`), footprint poses copied from `v4-4layer`. |
| `v1-*`, `keepout-in-footprint` | v1 generator (2026-09-24); 2-layer inputs got their poses from `v1-4layer`. |
| `esp-usb-placement` | v2 ESP32 variant (2026-09-25), free parts left at their initial stacked position. |

The ESP32 footprints point at a 3D model under `${KIPRJMOD}/../../3dmodels/`,
which doesn't exist here. That only affects 3D renders.

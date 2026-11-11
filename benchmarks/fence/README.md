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
- `esp-usb-placement`, the `v5-*` and the `v1-*` boards run by hand
  (commands below).
- The issue log says which pcb-maker behaviour each one exposes and when it
  counts as fixed.
- Keep the boards as they are (they are frozen snapshots). Report results
  here, in the results table.

## What is not working for us

What still stands between the fence project and a board it would order:

| # | What we need | State | Issue |
|---|---|---|---|
| A | **The fence on 2 layers at its 4-layer size.** GND pour on B.Cu. | **Not reached.** On v4 (31 mm) route mode, layout mode, constraint placement and pin swaps all stall on a handful of `ROW`/`MCU_ROW` nets at the MCU's and switch A's escape rings; 8 mm more height completes it with the GND pour in pieces. On v5 (34 mm) the opens moved to the LED row, where thirty row nets, the LED chain and the LEDs' supplies cross one 2.6 mm band: 5 open at 34 mm; complete at 44 and 46 mm, but only by cutting the pour into 10 to 41 pieces. The project stays on 4 layers. | 3, 17 |
| B | **A layout judged by routability.** On 2 layers the hand placement, the constraint placement and the pin swaps all end within one net of each other; nothing tells us whether a different placement would route, short of running it. | Missing: a "this cannot route on two layers" verdict, and swapping inside the move loop. The two-layer fence gets its moves, none of which is kept; four-layer boards with pours skip the moves above 5 % open. | |
| C | **Silkscreen.** | Mostly fixed: the layout places the Reference and Value fields where KiCad finds nothing; the v5 four-layer result has 4 findings left (J1's and J4's outlines over the edge). | 8 |

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
| A board-level rule area on In1.Cu named `GND plane: no tracks` | all 4-layer boards | issue 9 (fixed; the area is now redundant) |
| The ESP32 footprint's antenna keepout repeated as a board-level rule area | `v1-*`, `v4-*` `fence-esp32` (not `keepout-in-footprint`, not `esp-usb-placement`) | issue 1 (fixed; harmless where the module is fixed) |
| Chips, decoupling, regulator, pump and networks fixed by hand in `layout.json` | all | issue 2 (fixed; `layout-constraints.json` next to the v4 boards places them by intent instead) |
| A fixed row/pin assignment chosen by hand | all | issue 3 |

To test a fix, delete the rule area (or free the parts) in a copy.

## Benchmarks

Every directory is a KiCad 10 project (`<board_id>.kicad_pcb`, `.kicad_sch`,
`.kicad_pro`, project libraries) plus:
- `layout.json`: the `layout-kicad-board` config (fixed parts);
- `pin-swaps.json`: which pins are interchangeable (the format `docs/pin-swap.md` reads);
- on the v4 boards also `layout-swap.json` (the same `layout.json` plus `pin_swaps`), and `layout-constraints.json` + `constraints.json`: only the connectors and the row networks fixed, everything else placed by `edge`, `near` and `region` constraints.

| Directory | Board | State | What it measures | Pass |
|---|---|---|---|---|
| `v5-4layer/fence` | **current board (hardware v5):** 76.2 × 34 mm, CH32X035 only, 41 WS2812B-2020 LEDs (0.7 mm pads at 2.54 mm pitch along the 30-pin header), a mirror-symmetric 2×8 stack header on the back, three `keepout` constraints inline in `layout.json` | placed by hand + constraints, unrouted | Place + route on 4 layers; `router-seeds.json` (4 seeds) is what the project ships. | complete; no copper DRC error; **no via inside an LED pad** (the shipped board has 6; issue 13). Reference: 134/134, 196 vias, 3734 mm. |
| `v5-2layer/fence` | v5 generated with `--layers=2` at the same 34 mm: GND pour on B.Cu, JLCPCB's 0.127 mm track and clearance in the `.kicad_pro` (two tracks fit between neighbouring LEDs) | placed, unrouted | **Can the v5 board be routed on 2 layers at its 4-layer size, with a ground worth the name?** `layout-long.json` (3000 s) and `router-seeds.json` are the configs of the best run. | complete, with the B.Cu GND pour's largest piece covering most of the board (measure with `pour-pieces.py`; the 4-layer plane is 3 pieces, 78 %). Not reached (see Results). |
| `v5-2layer/fence-46mm` | the same 12 mm taller (`--extra=12`), `router-cut10.json` (`plane_cut_cost` 10, 0.4 mm power tracks) | placed, unrouted | Where it does complete, and what the pour looks like then. | complete **and** a pour in few pieces (see Results). |
| `v4-4layer/{fence,fence-esp32}` | hardware v4 | routed by pcb-maker (the reference result) | Route (designer placement) and place + route on 4 layers. | complete; no copper DRC error; no track on In1.Cu. Reference: 101/101 with 134 vias, 108/108 with 190 vias. |
| `v4-2layer/{fence,fence-esp32}` | v4 generated with `--layers=2` (GND pour on B.Cu instead of the plane), placement copied from `v4-4layer` | placed, unrouted | Can the same board be routed on 2 layers? | complete, with a GND pour that stays in one piece. Not reached (see Results). |
| `keepout-in-footprint` | v1 ESP32 variant, 2 layers, **without** the board-level copy of the antenna keepout; placement from `v1-4layer` | placed, unrouted | The router honours a keepout zone defined inside a footprint. | no copper in the antenna area; KiCad reports no `items_not_allowed`. |
| `esp-usb-placement` | v2 ESP32 variant (4 layers, 34 mm) with U3, J4, U2, C3, C4, C5, C6 free and stacked | unplaced | Edge, overhang and relative placement. See [esp-usb-placement/README.md](esp-usb-placement/README.md). | its README's table. |
| `v1-4layer/{fence,fence-esp32}` | v1 (with a 2×29 socket strip threaded by the 30 row tracks) | routed by pcb-maker | Older, harder 4-layer variant. | complete. v1 result: 121 / 150 vias. |
| `v1-2layer/{fence,fence-esp32}` | v1 on 2 layers, placement from `v1-4layer` | placed, unrouted | The hardest variant. | complete. `fence`: every connection, one starved thermal (see Results). |

The corpus entries are named `fence-*` in
[`../corpus/corpus.json`](../corpus/corpus.json):

```sh
cargo build --release
python3 benchmarks/corpus/run.py build/corpus-fence --only fence-v4-4l fence-esp32-v4-4l \
    fence-v4-2l fence-esp32-v4-2l fence-keepout
```

The v5 boards, by hand (the project's own commands are in
`~/programming/breadboard-enhancer/hw/README.md`):

```sh
b=benchmarks/fence/v5-4layer/fence
target/release/pcb-maker layout-kicad-board $b fence build/fence-v5-4l $b/router-seeds.json $b/layout.json
b=benchmarks/fence/v5-2layer/fence
target/release/pcb-maker layout-kicad-board $b fence build/fence-v5-2l $b/router-seeds.json $b/layout-long.json
python3 benchmarks/fence/pour-pieces.py build/fence-v5-2l/result/fence.kicad_pcb 34   # pieces of the GND pour
b=benchmarks/fence/v5-2layer/fence-46mm
target/release/pcb-maker layout-kicad-board $b fence build/fence-v5-2l-46 $b/router-cut10.json $b/layout-long.json
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

The newest pass over every board (2026-10-21, pcb-maker x1404, main
`20956b3`; `PCB_WORK_RATE=3.15e6,150e6`, 8 threads, one run at a time).
Vias in pads: pcbnew `pad.HitTest(via.GetPosition())` over SMD pads; pour:
`pour-pieces.py` (KiCad's fill).

| Board | Result |
|---|---|
| `v5-4layer/fence`, `router-seeds.json`, `layout.json` | **complete and clean:** 134/134, 181 vias, 3698 mm, 0 vias in pads; In1.Cu GND in 3 pieces, 78 % of the board; work 233 s. |
| `v5-2layer/fence` (34 mm), `router-seeds.json`, `layout-long.json` | **incomplete:** 129/134, 5 unconnected, no DRC error, 302 vias, 0 in pads; pour in 19 pieces, 9 % of the board; work 1165 s. |
| `v5-2layer/fence-46mm`, `router-cut10.json`, `layout-long.json` | **every connection, one starved thermal:** 134/134, 231 vias, 0 in pads; pour in 10 pieces, 13 %; work 283 s. With `add_pours[].keep_whole` on a strip across the middle band: complete and clean, 287 vias, pour 15 pieces, largest piece 373 mm². |
| `v5-2layer/fence` moved to 44 mm (the free-dimension search's choice; that input is not committed) | **complete and clean:** 134/134, 395 vias, 0 in pads; pour in 41 pieces, 42 %. |
| `esp-usb-placement` | 107/107, KiCad 0 unconnected, no `copper_edge_clearance`, 162 vias; C5 and C6 beyond their unreachable limit to U3 pin 3, J4 0.88 mm beyond its gap. |
| `v1-2layer/fence` | **every connection, one starved thermal:** 100/100, 293 vias, 0 in pads; pour in 16 pieces. |
| corpus `fence-v4-4l` | route mode 101/101, 131 vias, clean; layout 101/101, 131 vias, clean. |
| corpus `fence-esp32-v4-4l` | route mode 108/108, 176 vias, clean; layout 108/108, 171 vias, clean. |
| corpus `fence-v4-2l` | route mode 97/101, 5 unconnected; layout 92/101, 9 unconnected. |
| corpus `fence-esp32-v4-2l` | route mode 93/108, 16 unconnected; layout 96/108, 13 unconnected. |
| corpus `fence-keepout` | route mode 106/107, 2 unconnected, no `items_not_allowed`; layout 105/107, 2 unconnected. |

On 2 layers the opens are row nets (`ROWn` / `MCU_ROWn`) where they cross
between their networks and their switch. Whether a human could route the
two-layer boards at their four-layer size is open; the router completes
them only with more height and a ground pour in pieces.

## Issue log

Open:

| # | Severity | What happened | Status / workaround | Benchmark |
|---|---|---|---|---|
| 3 | high | **No pin swapping** (the fence's pins are largely interchangeable; see `pin-swaps.json`). | Implemented (`swap-kicad-pins`, `pin_swaps` in the route/layout config; [docs/pin-swap.md](../../docs/pin-swap.md)); board and schematic stay parity-clean, and `describe-kicad-board` lists `swap_candidates` by electrical type where the pads name no function. On the 2-layer boards it does not finish more nets than the hand-made assignment. | any board + its `pin-swaps.json`; compare vias, length and completion with the fixed assignment, and report the chosen mapping (the firmware takes its tables from it). |
| 7 | low | **Starved thermal relief** on a THT GND pad connected to a pour (1 of 4 spokes). | Cosmetic here. | `v1-2layer` |
| 8 | cosmetic | **Silkscreen.** Reference designators overlapped other silk and copper on every layout. | Mostly fixed: the layout places the Reference and Value fields by KiCad's rules (docs/placer.md); the v5 four-layer result has 4 findings left, the parts' own outlines over the edge. | any |
| 14 | medium | **Polishing moved board-level silkscreen text** (header labels, test-pad names) because the placer had put parts under it. | The placer treats the board's own texts as obstacles (docs/placer.md); not re-measured here. Workaround in `layout.json`: three `keepout` constraints over the label areas; the project puts the texts back afterwards (`finalize.py --labels`). | `v5-4layer/fence` without the `constraints` key in `layout.json` |
| 17 | medium | **No way to keep a 2-layer pour whole.** A board with GND in 37 pieces is "complete". | Mostly fixed: the verdict reports `pours` per zone and layer (pieces, largest share, coverage, cut), with a next action past 5 pieces; `add_pours[].keep_whole` keeps regions whole; `tuning.pour_integrity` (router config, two layers, default off) prices cuts by what they sever. Open: the other nets' tracks on B.Cu still mesh the fill into many islands whatever they cost. | `v5-2layer/fence`, `pour-pieces.py` |

Fixed:

| # | What happened | Fixed by |
|---|---|---|
| 1 | Keepout zones inside footprints were ignored while routing (the ESP32's antenna keepout). | The router honours footprint keepouts (`keepout-in-footprint`: no `items_not_allowed`). |
| 2, 11 | No placement constraints: decoupling, regulator and CC network far from their parts. | `constraints.json` ([docs/constraints.md](../../docs/constraints.md)) and the automatic decoupling pull. |
| 4 | Pad angles differed from KiCad's when a footprint's `(at …)` came after its pads. | The generator writes `(at)` first. |
| 5 | Overlapping fixed parts were not reported. | A warning names both parts, in the log and the verdict's next_actions. |
| 6 | No snapshot release. | `.github/workflows/release.yml` publishes the `snapshot` release on every push to main. |
| 9 | Signals were routed on a plane layer. | Exclusive planes: no track on In1.Cu. |
| 10 | `free` could not release a part matched by `fixed_patterns`; overhanging parts were fixed. | Constraints: a part named by a constraint may move; `edge` + `overhang` say where the antenna goes. |
| 12 | An edge-flush part ignored the project's copper-to-edge clearance. | Edge parts' pads keep the clearance. |
| 13 | Vias inside small SMD pads (the LEDs' GND pads). | No via in an SMD pad but an exposed one; `via_in_pad: true` restores it. |
| 15 | `keepout` constraints judged fixed parts. | A box over a held part allows footprints and is warned of. |
| 16 | `repair-kicad-silkscreen` rejected a `gr_rect` outline. | It reads `gr_rect` outlines. |

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

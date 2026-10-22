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

## What is not working for us (2026-10-07)

Re-run of every fence benchmark with pcb-maker `5061a55` (built 2026-10-07).
Since the September runs the tool gained footprint keepouts, exclusive
planes, placement constraints and pin swapping, and all four of those work
on these boards (details in the results and the issue log below). What still
stands between us and a board we would order:

| # | What we need | State | Issue |
|---|---|---|---|
| A | **The fence on 2 layers at 31 mm.** GND pour on B.Cu. | **Not routable by pcb-maker at 31 mm:** 96-97/101 (`fence`), 99-100/108 (`fence-esp32`); route mode, layout mode, constraint placement and pin swaps all stall on a handful of `ROW`/`MCU_ROW` nets at the MCU's and switch A's escape rings (failure-analysis F4). Height helps: +4 mm 100/101, +6 mm 99/101, **+8 mm (39 mm) complete** for `fence`, with 305 vias and the GND pour cut into 17 pieces (GND routed as tracks). ESP at +8 mm: also complete (108/108, 301 vias, GND pour in 16 pieces). Freerouting got to 8 open on the 31 mm board in September. We stay on 4 layers at 31 mm. | 3, F4, F5 |
| B | **A layout judged by routability.** On 2 layers the hand placement, the constraint placement and the pin swaps all end within one net of each other; nothing tells us whether a different placement would route, short of running it. | Missing: the congestion estimate / "this cannot route on two layers" verdict that F4 (4) proposes, and swapping inside the move loop. The coupled loop's "no moves above 5 % open" holds only on four layers with pours since 2026-10-07: the two-layer fence gets its moves (34 mm, 2026-10-21: 6 trials, none kept). On four layers a bounded round was tried (2026-10-21) and lost time for nothing (DSP-ADAU1452: each trial left 224-285 open against 100, 89 s of work), so the rule stays. | F4 |
| C | **Silkscreen.** Every result has 50-90 `silk_overlap` / `silk_over_copper` findings; `repair-kicad-silkscreen` on the v4 board changes nothing (56 before, 56 after, exit 1). | Open. We hide the row-network references and live with the rest. | 8 |
| D | **Edge-flush parts keep the project's copper-to-edge rule.** The constraint placer put the ESP32 module 0.5 mm too high: its top pads are 0.2 mm from the edge against the project's 0.3 mm rule (15 `copper_edge_clearance`). | **Fixed** (2026-10-21 run: no `copper_edge_clearance`). | 12 |
| E | **A warning for stacked fixed parts.** Two fixed parts at the same spot place and route with no message; the board just comes back with opens. | **Fixed** (2026-10-21, fence-benchmark: a warning naming both parts). | 5 |
| F | **`swap_candidates` finds nothing** on a board whose MCU pins are named `PA15`, `PB1`, … rather than `GPIO*`. The explicit `pin-swaps.json` works. | **Fixed** (2026-10-21, fence-benchmark: by electrical type where the pads name no function). | 3 |
| G | **A release to pin.** `releases/tag/snapshot` still does not exist; every run here used a local build. | A release workflow exists; first publication pending (2026-10-22: `.github/workflows/release.yml` replaces `snapshot` with every push to `main` that passes the gate, with `VERSION` and `SHA256SUMS`; `pcb-maker --version` and the verdict's `environment.pcb_maker_version` name the build; pinning is in the main README, "Try it"). | 6 |

Resolved since September, on these boards: footprint keepouts (1), signals
on the GND plane (9), placement constraints instead of hand placement (2,
11, 10), pin swapping (3, as a feature; it does not rescue the 2-layer
board).

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
| `v5-4layer/fence` | **current board (hardware v5, 2026-10-21):** 76.2 × 34 mm, CH32X035 only, 41 WS2812B-2020 LEDs (0.7 mm pads at 2.54 mm pitch along the 30-pin header), a mirror-symmetric 2×8 stack header on the back, three `keepout` constraints inline in `layout.json` | placed by hand + constraints, unrouted | Place + route on 4 layers; `router-seeds.json` (4 seeds) is what the project ships. | complete; no copper DRC error; **no via inside an LED pad** (the shipped board has 6; issue 13). Reference: 134/134, 196 vias, 3734 mm. |
| `v5-2layer/fence` | v5 generated with `--layers=2` at the same 34 mm: GND pour on B.Cu, JLCPCB's 0.127 mm track and clearance in the `.kicad_pro` (two tracks fit between neighbouring LEDs) | placed, unrouted | **Can the v5 board be routed on 2 layers at its 4-layer size, with a ground worth the name?** `layout-long.json` (3000 s) and `router-seeds.json` are the configs of the best run. | complete, with the B.Cu GND pour's largest piece covering most of the board (measure with `pour-pieces.py`; the 4-layer plane is 3 pieces, 78 %). Best so far: 7 open, pour in 43 pieces at 25 %. |
| `v5-2layer/fence-46mm` | the same 12 mm taller (`--extra=12`), `router-cut10.json` (`plane_cut_cost` 10, 0.4 mm power tracks) | placed, unrouted | Where it does complete, and what the pour looks like then. | complete (it is: 134/134, 547 vias) **and** a pour in few pieces (it is not: 37 pieces, 27 %). |
| `v4-4layer/{fence,fence-esp32}` | hardware v4 | routed by pcb-maker (the reference result) | Route (designer placement) and place + route on 4 layers. | complete; no copper DRC error; no track on In1.Cu. Reference: 101/101 with 134 vias, 108/108 with 190 vias. |
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

The v5 boards, by hand (pcb-maker `36f24ba`, 2026-10-21; the project's own
commands are in `~/programming/breadboard-enhancer/hw/README.md`):

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

| 2026-10-07 | **re-run, pcb-maker `5061a55`** (default budgets: layout 1500 s) | |
| 2026-10-07 | `v4-4layer/fence` **without** its `GND plane: no tracks` rule area, route mode | **complete and clean:** 101/101, 134 vias, 2962 mm, 157 s; the exclusive-plane rung keeps In1.Cu free of tracks by itself (issue 9 fixed). |
| 2026-10-07 | `keepout-in-footprint`, route mode | 106/107, 314 vias; **no `items_not_allowed`** (issue 1 fixed); 1 unconnected, 5 starved thermals (the hard v1 2-layer board). |
| 2026-10-07 | `esp-usb-placement` with its `constraints.json` (U3, J4, U2, C3-C6 free) | **complete,** 107/107, 167 vias, 177 s. U3 at (6.8, 2.4), J4 at (4.1, 20.1) below it, U2 at (14.5, 15.5) between: the designer's answer within 0.5 mm. Missed: C5/C6 within 6 mm of U3 pin 3 (2.4 and 2.6 mm over; the pin is inside the courtyard, so unreachable), J4's gap 0.9 mm over. But U3 sits 0.5 mm higher than the designer put it: 15 `copper_edge_clearance` (pads 0.2 mm from the edge against the project's 0.3 mm; issue 12). |
| 2026-10-07 | `v4-4layer/fence` with `layout-constraints.json` (only J*, RN1-8 fixed; the chips, regulator, decoupling and pump placed by constraints) | **complete and clean:** 101/101, 144 vias (134 by hand), 3046 mm, 128 s; every constraint met. |
| 2026-10-07 | `v4-4layer/fence-esp32` with `layout-constraints.json` | **complete and clean:** 108/108, 191 vias (190 by hand), 3184 mm, 160 s. Missed soft constraints: C5 9.4 mm and C6 3.4 mm beyond their 6 mm to U3 pin 3, C7 6.2 mm beyond RN11, C4 2.3 mm beyond U2. |
| 2026-10-07 | `v4-2layer/fence`, route mode (designer placement) | **incomplete:** 96/101, 7 unconnected, 238 vias (September: 91/101). |
| 2026-10-07 | `v4-2layer/fence`, layout mode, `layout.json` (hand pins) | **incomplete:** 96/101, 6 unconnected (`MCU_LINK1`, `MCU_ROW5`, `MCU_ROW8`, `ROW10`, `ROW13`), 271 vias. "41 open after the first route: no moves, the final ladder gets the time". |
| 2026-10-07 | same, `layout-swap.json` (pin swaps) | **incomplete:** 97/101, 5 unconnected (`MCU_ROW11`, `ROW10`, `ROW15`, `XP_STB_A`), 271 vias. |
| 2026-10-07 | same, `layout-constraints.json` (chips and passives placed by constraints) | **incomplete:** 92/101, 19 unconnected, 249 vias. "55 open after the first route: no moves": with more than 5 % open the coupled loop skips its moves, so on exactly the board where a different placement might route, no placement change is tried. |
| 2026-10-07 | same, 4 mm taller (`--extra=4`, 35 mm board, hand pins) | **incomplete by one:** 100/101, 1 unconnected, 277 vias, 3842 mm (September: 88/100). |
| 2026-10-07 | same, 4 mm taller, pin swaps | **incomplete:** 98/101, 4 unconnected, 258 vias. |
| 2026-10-07 | same, 6 mm taller (37 mm board, hand pins) | **incomplete:** 99/101, 4 unconnected, 262 vias. |
| 2026-10-07 | same, 8 mm taller (39 mm board, hand pins) | **complete:** 101/101, 0 unconnected, 305 vias, 4162 mm, 1017 s; pour nets as tracks, the GND pour on B.Cu in 17 pieces; 3 starved thermals (September needed 12 mm more and left the pour in 127 pieces). |
| 2026-10-07 | `v4-2layer/fence-esp32`, layout mode, hand pins | **incomplete:** 99/108, 15 unconnected (`MCU_ROW3/7/10`, `MCU_LINK0`, `SCL`, `SWDIO`, `BUS6`, `VDD`, `XP_RST`), 248 vias (September: 81/107). |
| 2026-10-07 | same, 8 mm taller (39 mm board, hand pins) | **complete:** 108/108, 0 unconnected, 301 vias, 4032 mm, 1591 s; pour nets as tracks, GND pour in 16 pieces, no copper DRC error. |
| 2026-10-07 | same, pin swaps | **incomplete:** 100/108, 11 unconnected (BUS2, BUS3, ESP_IO9, MCU_ROW6, MCU_ROW7, ROW13, ROW16, SCL), 256 vias. |

(v3 and v4 differ only in the negative supply: a TPS60401 in v3, a diode
charge pump in v4. The v4 2-layer rows above are route mode with the designer's
placement.)

On 2 layers the opens are almost always row nets (`ROWn` / `MCU_ROWn`)
around chip A and the MCU: exactly where pin swapping (issue 3) would help
most. Whether a human could route the 31 mm board on 2 layers is open.

| 2026-10-21 | **v5, pcb-maker `36f24ba`** (head of that day) | |
| 2026-10-21 | `v5-4layer/fence`, layout, default router | **complete and clean:** 134/134, 182–191 vias, 3759–3809 mm, 150–280 s; byte-identical on three repeats of one input. 3 to 10 vias inside LED GND pads depending on the input's details. |
| 2026-10-21 | same, `router-seeds.json` (4 seeds) | **complete:** 134/134, 196 vias, 3734 mm; 8 vias in pads (6 LED pads, 2 test pads). Shipped. |
| 2026-10-21 | `v5-2layer/fence` (34 mm), 0.2 mm rules, default | **incomplete:** 114/134, 32 unconnected, 322 vias (28 in pads), GND pour cut by 1453 mm. |
| 2026-10-21 | same, 0.127 mm rules, `total_seconds` 3000 | **incomplete:** 10 unconnected, 281 vias; stopped at 759 s of work ("the finer lattice (0.075 mm) was not tried: projected 367 s of routing exceeds refine_budget_seconds (240 s)"). |
| 2026-10-21 | same, plus `{"seeds": 3, "refine_budget_seconds": 1500}` | **incomplete:** 7 unconnected (`LED_CHAIN16`, `MCU_ROW5/6/11/12`, `ROW11`, `BUS4`, `VBUS` conflicted), 421 vias, 12 in pads, work 364 s; **pour in 43 pieces, 638 of 2591 mm²**. Hot spots: the 2.6 mm LED band on B.Cu, x 17–35 mm. |
| 2026-10-21 | v5 2 layers, 4 / 6 mm taller, 0.127 mm, 2 seeds | 3 unconnected (pour 28 pieces, 12 %); 1 unconnected (pour 30 pieces, 36 %, 31 vias in pads). |
| 2026-10-21 | v5 2 layers, 8 / 12 mm taller, `router-cut10.json` | 1 unconnected (19 pieces, 12 %); **complete at 46 mm**: 134/134, 547 vias (19 in pads), 5399 mm, work 613 s, **pour in 37 pieces, 959 of 3505 mm²**. `plane_cut_cost` 10 cut the pour as much as the default did (2088–2118 mm against 1594–1908 mm). |

| 2026-10-21 | **benchmark pass, pcb-maker x1404 (main `20956b3`) and x1462 (branch fence-benchmark, `67fa8f4`)**; `PCB_WORK_RATE=3.15e6,150e6`, 8 threads, one run at a time. Vias in pads: pcbnew `pad.HitTest(via.GetPosition())` over SMD pads. Pour: `pour-pieces.py` (KiCad's fill), and the new verdict line `pours` (our pour model) in brackets | |
| 2026-10-21 | `v5-4layer/fence`, `router-seeds.json`, `layout.json`, x1404 | **complete and clean:** 134/134, 181 vias, 3698 mm, **0 vias in pads**; In1.Cu GND in 3 pieces, 2019 + 2 + 2 of 2591 mm² (78 %); work 233 s, wall 144 s. |
| 2026-10-21 | same, x1462 | **complete and clean:** 134/134, 179 vias, 3930 mm, 0 in pads; In1.Cu 3 pieces, 2038 mm² (79 %) [1 piece, 2065 mm²]; work 312 s, wall 197 s. Differs from x1404 because the `GND plane: no tracks` rule area no longer hides the plane's fill from the router's pour model (x1404 routed as if In1.Cu had no fill). |
| 2026-10-21 | `v5-2layer/fence` (34 mm), `router-seeds.json`, `layout-long.json` (3000 s), x1404 | **incomplete:** 129/134, 5 unconnected, no DRC error, 302 vias, 3828 mm, 0 in pads; pour 19 pieces, largest 60, 230 of 2591 mm² (9 %) [23 pieces, 195 mm², cut by 1712 mm]; work 1165 s, wall 698 s. The ladder ran every rung at the 0.05 mm lattice (best: tracks, 12 open by the router's count) and ended there, not on a budget. Moves ran (6 trials, none kept). x1462: the same board byte for byte. |
| 2026-10-21 | `v5-2layer/fence-46mm`, `router-cut10.json`, `layout-long.json`, x1404 | **every connection, one starved thermal:** 134/134, KiCad 0 unconnected, `starved_thermal` 1, 231 vias, 4475 mm, 0 in pads; pour 10 pieces, largest 293, 441 of 3505 mm² (13 %) [9 pieces, 294 / 441 mm²]; work 283 s, wall 377 s. |
| 2026-10-21 | same, x1462, `add_pours` GND on B.Cu with `keep_whole` U1, U4, U5 | **incomplete:** 128/134, 6 unconnected, 1 starved, 370 vias; pour 18 pieces, largest 408, 994 mm² (28 %); work 1706 s. |
| 2026-10-21 | same, `keep_whole` U1, U4, U5 and a strip x 2–74.2, y 27–28.5 | **incomplete:** 126/134, 9 unconnected, 2 starved, 401 vias; pour 12 pieces, **largest 719**, 1163 mm² (33 %). |
| 2026-10-21 | same, `keep_whole` the strip alone | **complete and clean:** 134/134, 287 vias, 4771 mm, 0 in pads; pour 15 pieces, **largest 373, 657 mm² (19 %)** [14 pieces, 378 / 667 mm²]; work 753 s, wall 451 s. The strip carries the GND pour across the board's middle band, where every row net crosses. |
| 2026-10-21 | `esp-usb-placement`, x1404 | 107/107, KiCad 0 unconnected, **no copper_edge_clearance** (issue 12 gone), 162 vias, 3271 mm, 0 in pads; constraints missed as on 2026-10-07: C5 2.61 mm and C6 2.40 mm beyond U3 pin 3, J4 0.88 mm beyond its gap (status incomplete for them); work 163 s, wall 132 s. |
| 2026-10-21 | `v1-2layer/fence`, x1404 | **every connection, one starved thermal:** 100/100 (2026-09-24: 92/100), KiCad 0 unconnected, 293 vias, 3767 mm, 0 in pads; pour 16 pieces, largest 108, 282 of 2362 mm²; work 1686 s, wall 690 s. |
| 2026-10-21 | corpus `fence-v4-4l`, x1404 | route mode 101/101, 131 vias, 3194 mm, clean, 89 s; layout 101/101, 131 vias, 3117 mm, clean, 224 s. |
| 2026-10-21 | corpus `fence-esp32-v4-4l`, x1404 | route mode 108/108, 176 vias, clean, 103 s; layout 108/108, 171 vias, clean, 160 s. |
| 2026-10-21 | corpus `fence-v4-2l`, x1404 | route mode 97/101, 5 unconnected, 1 starved, 279 vias, 308 s; layout 92/101, 9 unconnected, 280 vias, 567 s. |
| 2026-10-21 | corpus `fence-esp32-v4-2l`, x1404 | route mode 93/108, 16 unconnected, 1 starved, 248 vias, 388 s; layout 96/108, 13 unconnected, 273 vias, 633 s. |
| 2026-10-21 | corpus `fence-keepout`, x1404 | route mode 106/107, 2 unconnected, 1 starved, 344 vias, **no `items_not_allowed`**, 312 s; layout 105/107, 2 unconnected, 2 starved, 417 vias, 508 s. |
| 2026-10-21 | `v5-2layer/fence`, free dimension (bottom edge 34-60 mm, step 2, prefer_below 100, band 10, auto router) | **not rerun:** the search runs under the machine's tier lock, which had 47 runs waiting; the x1420 search's choice stands (44 mm, complete and clean, free-dimension changelog of 2026-10-21). Its 44 mm input (`free-dimension/44mm-confirm/input`, the board moved as the search moves it) laid out singly below. |
| 2026-10-21 | that 44 mm board, its confirmation's `layout.json` (1500 s), auto router, x1404 | **complete and clean:** 134/134, 395 vias, 4649 mm, 0 in pads; pour 41 pieces, largest 389, 1425 of 3353 mm² (42 %) [43 pieces, 341 / 1430 mm²]; work 285 s, wall 201 s. |
| 2026-10-21 | same, x1462, `keep_whole` a strip x 2–74.2, y 28.25–29.75 | **complete and clean:** 134/134, 495 vias, 4812 mm, 0 in pads; pour 39 pieces, **largest 630**, 1284 mm² (38 %) [43 pieces, 624 / 1308 mm²]; work 222 s, wall 158 s. |

On v5 the opens moved from the chips' escape rings (v4) to the LED row:
thirty row nets, the LED chain's thirty diagonals and the LEDs' 5 V and GND
all cross one 2.6 mm band between the 30-pin header and the resistor
networks. Whether a human could do it at 34 mm is as open as before; the
46 mm result says the router can, but only by shredding the ground.

## Issue log

| # | Date | Severity | What happened | Status / workaround | Benchmark |
|---|---|---|---|---|---|
| 1 | 2026-09-24 | high | **Keepout zones inside footprints are ignored while routing.** The ESP32-C3-MINI-1 footprint carries its antenna keepout (tracks, vias, pads, pours not allowed) as a footprint zone. pcb-maker routed 13 tracks through it. Only KiCad's DRC caught it (`items_not_allowed`); pcb-maker reported 92/92 and exit 0. | **Fixed** (2026-10-07 re-run: no `items_not_allowed` on `keepout-in-footprint`). The board-level copy stays in the v4 boards (harmless: it lies outside the outline) and was removed from `esp-usb-placement`, where it did not move with the module. | `keepout-in-footprint` |
| 2 | 2026-09-24 | high | **No placement constraints.** The placer only optimises wire length. Decoupling capacitors ended up far from their chips (the ESP32's C5/C6 on the far side of the board); the regulator drifted away from the USB connector and the module it feeds. `docs/placer.md`: "No constraint language yet (regions, groups, alignment, decoupling proximity, keep-near-edge)". | **Fixed** by `constraints.json` ([docs/constraints.md](../../docs/constraints.md)) and the automatic decoupling pull: with `layout-constraints.json` both v4 boards place and route completely on 4 layers (2026-10-07). | `esp-usb-placement`; `v4-4layer/*` with `layout-constraints.json` |
| 3 | 2026-09-24 | high | **No pin swapping.** Most of the fence's pins are interchangeable (see `pin-swaps.json`): any CH32X035 GPIO can serve a row, any resistor-network element any channel, any CH446Q X port any row of that chip, any ESP32-C3 GPIO the link lines. The fixed assignment is a hand-made guess at a crossing-free order. | Implemented 2026-09-25 (`swap-kicad-pins`, `pin_swaps` in the route/layout config; [docs/pin-swap.md](../../docs/pin-swap.md)); board and schematic stay parity-clean. On the v4 2-layer boards it does not finish more nets than the hand-made assignment (97/101 against 96/101 on 2026-10-07). `describe-kicad-board` listed no `swap_candidates` for the fence: its pads carry no pin functions (the names are in the schematic only). **Fixed** 2026-10-21 (fence-benchmark): a part whose pads name no function is listed by its pins' electrical types and symbol unit (`by: electrical type`): on v5 U1 (44 bidirectional pins), U4 and U5 (24 passive X/Y ports each), RN1-RN8, with pad numbers as the `swappable` globs; boards with `GPIO*` functions keep their lists (short tier: all 12 unchanged). | any board + its `pin-swaps.json`; compare vias, length and completion with the fixed assignment, and report the chosen mapping (the firmware takes its tables from it). |
| 4 | 2026-09-24 | medium | **Pad angles differ from KiCad's when a footprint's `(at …)` comes after its pads.** KiCad stores pad angles as absolute values and converts them with the rotation it has read so far. pcb-maker's model and KiCad's disagreed after rotating such footprints: 30 `shorting_items` inside resistor arrays, visible only in KiCad's DRC. | The generator now writes `(at)` first. pcb-maker could reject such files or read them like KiCad. | none kept |
| 5 | 2026-09-24 | low | **Overlapping fixed parts are not reported.** A too-broad `fixed_patterns` glob fixed six resistors still stacked at their initial position; the layout ended with 12 opens and no hint. | **Fixed** (2026-10-21, branch fence-benchmark): parts the placer keeps where they are that lie on top of each other (courtyards on one side sharing half the smaller one, or pads meeting on a copper layer) are warned of before the run, in the log and first in the verdict's next_actions: `fixed parts C1 and C2 overlap (courtyards and pads on F.Cu)`; unit test with two fixed parts at one spot. | none kept |
| 6 | 2026-09-24 | low | **No snapshot release.** The README links `releases/tag/snapshot`, but it doesn't exist. | A release workflow exists; first publication pending (2026-10-22: the old snapshot workflow kept the release current from 2026-10-07 until `21194a9` on 2026-10-21, then stopped: the macOS build failed and held every release; `release.yml` fixes the build and no longer lets macOS hold Linux). Built from the checkout until then. | – |
| 7 | 2026-09-24 | low | **Starved thermal relief** on a THT GND pad connected to a pour (1 of 4 spokes). | Cosmetic here. | `v1-2layer` |
| 8 | 2026-09-24 | cosmetic | **Silkscreen is not considered.** Reference designators overlap other silk and copper on every layout (`silk_overlap`, `silk_over_copper`: 56 and 94 findings on v4). | Open. `repair-kicad-silkscreen` on the v4 board: 56 findings before, 56 after, exit 1 (2026-10-07). | any |
| 9 | 2026-09-25 | medium | **Signals are routed on a plane layer.** In1.Cu is typed `power` and holds a full GND zone; pcb-maker still routed signal tracks on it, leaving large voids in the plane. The router config has no per-layer or per-net layer restrictions. | **Fixed** (exclusive planes, 2026-10-07: `v4-4layer/fence` without its rule area routes 101/101 with no track on In1.Cu). The rule area stays in the generator for KiCad's benefit; it no longer matters to pcb-maker. | `v4-4layer` minus the rule area |
| 10 | 2026-09-25 | low | **Placement config:** `free` cannot release a part matched by `fixed_patterns`. A footprint that deliberately overhangs the outline (the ESP32's antenna) is treated as fixed by default (`on_edge`), and the placer has no notion of which part of a footprint may overhang. | Superseded by constraints: a part named by a constraint may move, and `edge` + `overhang` say where the antenna goes (2026-10-07: U3 placed flush with the antenna beyond the top edge). | `esp-usb-placement` |
| 11 | 2026-09-25 | low | **Proximity again (issue 2):** the placer put the USB-C CC network (RN9) 25 mm from the connector's CC pins, and a charge pump's output capacitor 18 mm from the pump. | **Fixed** as issue 2: `near` constraints (and the automatic decoupling pull) keep them at their pins. | as issue 2 |

| 12 | 2026-10-07 | medium | **An edge-flush part ignores the project's copper-to-edge clearance.** `esp-usb-placement` with `{"edge": "left", "flush": true, "overhang": {"edge": "top"}}`: U3 is placed with its origin at y = 2.4 mm from the top edge; its top row of pads ends 0.2 mm from the edge, against the project's `min_copper_edge_clearance` of 0.3 mm (15 `copper_edge_clearance`). The designer's 2.9 mm keeps the rule. `docs/constraints.md` says pads of edge parts keep that clearance. | **Fixed** (2026-10-21 run with x1404: no `copper_edge_clearance` on `esp-usb-placement`; the overhanging part's pads keep the clearance since 2026-10-07, `5f6dc2f`). | `esp-usb-placement` |

| 13 | 2026-10-21 | high | **Vias inside small SMD pads.** On v5 the router puts GND vias inside the 0.7 mm GND pads of 2 × 2 mm LEDs (3 to 28 of them depending on the run; 6 on the shipped board). `score-kicad-board` only prices them ("left open they wick solder; filled and capped they cost $17-50"); no setting forbids a via under a pad, and `seeds` choose by opens, vias and copper, so a seed with vias in pads wins. For a stencil-and-hot-plate assembly such a via empties the pad. | **Fixed** (x1404, `ded766d`: no via in an SMD pad but an exposed one, `via_in_pad: true` restores it). 2026-10-21 pass: 0 vias in SMD pads by `HitTest` on every fence result (v5 four layers, v5 two layers at 34 and 46 mm, esp-usb-placement, v1 two layers). | `v5-4layer/fence`: count with pcbnew `pad.HitTest(via.GetPosition())` |
| 14 | 2026-10-21 | medium | **Polishing moves board-level silkscreen text, and the placer does not see it.** Header labels (`3V3  VDD  5V`) and test-pad names moved up to 7.5 mm, one of them under the wrong socket, because the placer had put a button and two resistors under them. | Open in this pass (its fix is the placer side's, "fence report: placer side" in docs/changelog.md, not measured here). Workaround in `layout.json`: three `keepout` constraints over the label areas, and the project puts the texts back afterwards (`finalize.py --labels`). Wanted: a silkscreen text's box is a placement obstacle; input text is never moved. | `v5-4layer/fence` without the `constraints` key in `layout.json` |
| 15 | 2026-10-21 | low | **`keepout` constraints become KiCad rule areas that also judge fixed parts.** Any fixed part whose courtyard touches a box is an `items_not_allowed` error after the run (12, then 4, on v5 until the boxes were shrunk to 0.5 mm strips). | **Fixed** (x1404, `ded766d`): a box over a held part is written with footprints allowed and warned of at input time (log and next_actions); the 2026-10-21 v5 four-layer results have no `items_not_allowed`. | `v5-4layer/fence`, widen a keepout in `layout.json` |
| 16 | 2026-10-21 | low | **`repair-kicad-silkscreen` rejects a `gr_rect` outline** (`AssertionError: Rectangular board required`); the v5 boards' Edge.Cuts is one `gr_rect`. | **Fixed** (x1404): `repair-kicad-silkscreen` on the 2026-10-21 v5 four-layer result runs on the `gr_rect` outline (12 findings left, exit 1; no AssertionError). | `v5-4layer/fence` result |
| 17 | 2026-10-21 | medium | **No way to keep a 2-layer pour whole, and no verdict on it.** `plane_cut_cost` 10 did not reduce the cutting; `exclusive_planes` applies to inner layers only. A board with GND in 37 pieces at 27 % copper is "complete". `refine_budget_seconds` (240 s) also stops the ladder long before `total_seconds` (759 s used of 3000). | **Mostly fixed.** Budgets (x1404, `ded766d`): a layout's ladder budget is its work left and `refine_budget_seconds` 0.4 of it (34 mm, 2026-10-21: the 0.05 mm lattice ran). Verdict (fence-benchmark): `pours` per zone and layer (pieces, largest share, coverage, cut), from the router's pour model after the final route, a next action past 5 pieces; within 2 % of `pour-pieces.py`'s copper and about 10 % of its pieces on most boards (34 mm x1404: 195 against 230 mm²). Keeping it whole: `add_pours[].keep_whole` (parts or boxes); at 46 mm a strip across the middle band kept the board complete and clean and grew the largest piece 293 to 373 mm², the copper 441 to 657 mm², and at 44 mm (the free dimension's size) kept it complete and clean with the largest piece 389 to 630 mm²; regions under the chips lost completeness (6 and 9 open). Open: fewer pieces (15 against 10 at 46 mm, 39 against 41 at 44). | `v5-2layer/fence`, `pour-pieces.py` |

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

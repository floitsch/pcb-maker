# Board corpus and results

`benchmarks/corpus/run.py` copies each board, strips every track, via and
copper zone, and then runs two modes:

- **Route** keeps the designer's placement and tests the router alone.
- **Place + route** re-places every footprint that is not mechanically fixed
  (locked, without nets, or on the board edge) and then routes, using the
  coupled loop from [placer.md](placer.md).

Rules are read from each board's `.kicad_pro`; nothing is configured per
board. A result counts as clean when native KiCad reports no unconnected items
and no DRC error other than silkscreen or library cosmetics, *beyond what the
stripped source already had* (for example Olimex's fiducials next to its
mounting holes), and the internal exact verifier reports nothing.

```sh
benchmarks/corpus/fetch.sh                      # downloads the KiCad demos
cargo build --release
python3 benchmarks/corpus/run.py build/corpus   # add --only NAME... or --skip-layout
```

## Results (2026-09-22, one thread)

Tracks and vias are stripped; copper pours stay (where copper is poured is a
design decision like the placement). `pours connect` means pads reached their
pour through stubs, vias and stitching; `pours tracks` means the pour nets were
routed as tracks because connecting through the pour left something open (the
router tries both and keeps the better board). Route times are routing only;
place + route times are the whole coupled loop including its routing probes
and the final native check. "Clean" means no unconnected items and no copper
DRC error beyond what the stripped source or the designer's own board has.

| Board | Reference copper | Route (designer placement) | Place + route (automatic) |
| --- | --- | --- | --- |
| ecc83 | 59 tracks, 0 vias, 1 zones | 9/9, 0 vias, 160 mm, 0.08 s, pours connect — clean | 9/9, 0 vias, 140 mm, 5.8 s, pours connect — clean |
| hierarchy | 364 tracks, 0 vias, 166 zones | 50/50, 0 vias, 986 mm, 0.62 s, pours connect — clean | 50/50, 0 vias, 1119 mm, 10.0 s, pours connect — clean |
| pic | 370 tracks, 6 vias, 1 zones | 34/34, 0 vias, 1944 mm, 2.96 s, pours tracks — clean | 34/34, 6 vias, 2100 mm, 23.0 s, pours connect — clean |
| interf-u | 731 tracks, 84 vias, 1 zones | 110/110, 52 vias, 4610 mm, 10.53 s, pours tracks — clean | 110/110, 63 vias, 4674 mm, 361.8 s, pours tracks — clean |
| olimex-c3 | 768 tracks, 88 vias, 160 zones | 34/34, 50 vias, 904 mm, 1.43 s, pours tracks — clean | 34/34, 61 vias, 912 mm, 28.8 s, pours tracks — clean |
| dut-c3 | 321 tracks, 57 vias, 2 zones | 42/42, 29 vias, 1055 mm, 2.16 s, pours connect — clean | 42/42, 23 vias, 956 mm, 17.3 s, pours connect — clean |
| dut-c6 | 359 tracks, 68 vias, 2 zones | 42/42, 34 vias, 1031 mm, 3.25 s, pours connect — clean | 42/42, 24 vias, 1020 mm, 14.4 s, pours connect — clean |
| dut-s2 | 432 tracks, 83 vias, 2 zones | 46/46, 45 vias, 1211 mm, 3.72 s, pours connect — clean | 46/46, 51 vias, 1356 mm, 24.3 s, pours connect — clean |
| dut-s3 | 392 tracks, 64 vias, 2 zones | 46/46, 44 vias, 1129 mm, 3.19 s, pours connect — clean | 46/46, 42 vias, 1144 mm, 17.7 s, pours connect — clean |
| dut-esp32 | 345 tracks, 64 vias, 2 zones | 46/46, 41 vias, 1138 mm, 3.5 s, pours connect — clean | 46/46, 43 vias, 1446 mm, 29.7 s, pours tracks — clean |
| ngdevkit (designer never finished it) | 115 tracks, 29 vias, 2 zones | 163/180, 1079 vias, 18 669 mm, 824 s — **190 unconnected** (the 207-pad and 90-pad pours) | timed out |
| sonde-xilinx | 208 tracks, 3 vias, 1 zones | 26/26, 1 vias, 515 mm, 0.43 s, pours connect — clean | 26/26, 1 vias, 789 mm, 15.3 s, pours tracks — clean |
| multichannel | 576 tracks, 29 vias, 2 zones | 79/79, 25 vias, 2056 mm, 5.52 s, pours connect — clean | 79/79, 39 vias, 2077 mm, 52.5 s, pours tracks — starved_thermal×1 |
| stickhub | 1113 tracks, 87 vias, 5 zones | 43/45, 43 vias, 782 mm, 2.9 s — **9 unconnected** (17-pin +3.3 V) | **placement not legal** |
| coldfire (4 layers, 209 nets) | 2935 tracks, 253 vias, 3 zones | 208/209, 177 vias, 9111 mm, 204 s — **12 unconnected** (GND, inner plane islands) | 208/209, 207 vias, 8890 mm, 1189 s — 12 unconnected |
| video (4 layers, 371 nets, 189 footprints) | 7932 tracks, 808 vias, 2 zones | 364/371, 542 vias, 34 746 mm, 406 s — **8 unconnected** | timed out |
| openair-max (4 layers, 120 nets, 210 footprints) | 1638 tracks, 410 vias, 34 zones | 120/120, 124 vias, 4315 mm, 58 s — clean | **placement not legal** |
| tiny_tapeout (4 layers; its source fails KiCad DRC: 8 clearance errors, 3 shorts) | 2143 tracks, 405 vias, 2 zones | 81/108, 141 vias, 7029 mm, 143 s — 40 unconnected | **placement not legal** |

Thirteen boards are complete and clean in route mode, twelve in both modes.
The four-layer boards are new: the adapter now reads the copper stack from the
board and routes with through vias on all layers; openair-max (210 footprints)
is complete and clean, ColdFire and video are one and seven nets short.

## What the failures say

- **StickHub** (16 x 40 mm, two-sided SMD, 0.15 mm rules): the 17-pin +3.3 V
  net does not fit once the other nets are in; its designer used small +3.3 V
  pour regions, which the router treats as routing. The placer needs side
  assignment for a board this dense.
- **ngdevkit** (174 x 134 mm, 186 footprints, about 1070 pads): the 207-pad
  ground pour and the 90-pad 3.3 V pour are cut into 1500 pieces by the signal
  routing and cannot all be stitched. Pour nets need to be planned first, with
  pour continuity as a resource in the tile graph.
- **ColdFire / video** (4 layers): the remaining opens are ground pads whose
  inner-plane island cannot be stitched back to the main plane; same cause as
  ngdevkit, on inner layers.
- **Automatic placement on dense two-sided boards** (StickHub, openair,
  tiny_tapeout) cannot be legalized without side assignment.

## Coupled loop (2026-09-22, later the same day)

`layout-kicad-board` now nudges congested footprints and reroutes
incrementally instead of re-placing from scratch
([coupling.md](coupling.md)). Place + route results, same boards:

| Board | Halo rounds (old) | Incremental moves (new) |
| --- | --- | --- |
| ecc83 | 9/9, 0 vias, 140 mm, 6 s | 9/9, 0 vias, 140 mm, 6 s |
| hierarchy | 50/50, 0 vias, 1119 mm, 10 s | 50/50, 0 vias, 1194 mm, 29 s |
| pic | 34/34, 6 vias, 2100 mm, 23 s | 34/34, 4 vias, 1920 mm, 480 s |
| interf-u | 110/110, 63 vias, 4674 mm, 362 s | 110/110, 56 vias, 4399 mm, 235 s |
| olimex-c3 | 34/34, 61 vias, 912 mm, 29 s | 34/34, 65 vias, 960 mm, 19 s |
| dut-c3 | 42/42, 23 vias, 956 mm, 17 s | 42/42, 22 vias, 991 mm, 67 s |
| dut-c6 | 42/42, 24 vias, 1020 mm, 14 s | 42/42, 24 vias, 985 mm, 35 s |
| dut-s2 | 46/46, 51 vias, 1356 mm, 24 s | 46/46, 44 vias, 1317 mm, 205 s |
| dut-s3 | 46/46, 42 vias, 1144 mm, 18 s | 46/46, 31 vias, 1130 mm, 64 s |
| dut-esp32 | 46/46, 43 vias, 1446 mm, 30 s | 46/46, 42 vias, 1217 mm, 22 s |
| sonde-xilinx | 26/26, 1 via, 789 mm, 15 s | 26/26, 1 via, 789 mm, 13 s |
| multichannel | 79/79, 39 vias, 2077 mm, 52 s | 79/79, 42 vias, 1664 mm, 14 s |

All clean in native KiCad (multichannel keeps one starved thermal spoke in
both). Fewer vias or less copper on nine of twelve boards; the price is time on
boards where many nudges are tried.

## Head-to-head with Freerouting and tscircuit (2026-09-22)

`run.py --freerouting benchmarks/corpus/freerouting.json --tscircuit` routes
every two-layer board with three routers on the same *cold* board (tracks,
vias and pours stripped, since the Specctra exchange carries no pours), with
the same KiCad rules, and judges all three with the same native KiCad DRC
minus what the source already had:

- **pcb-maker**, one thread (plus parallel batches where nets are disjoint).
- **Freerouting 2.2.4** through the project's adapter
  (`route-kicad-board-freerouting`), which translates the board minimum into
  the DSN class rules and the edge clearance; one thread, optimizer off.
- **tscircuit capacity-autorouter 0.0.919** (the other actively developed
  open-source PCB autorouter) through `benchmarks/tscircuit/route_dsn.mjs`,
  which turns the same DSN into its SimpleRouteJson (pads as rectangles, nets
  as pad centres, the board's width, clearance and via rules through the
  solver's fields) and writes a Specctra session for the same import.

"Bad" counts KiCad DRC errors plus unconnected items.

| Board | pcb-maker | Freerouting | tscircuit |
| --- | --- | --- | --- |
| ecc83 | 9/9, 0 vias, 249 mm, 0.1 s, clean | complete, 0 vias, 253 mm, 1.5 s, clean | complete, 0 vias, 273 mm, 3.4 s, **7 bad** |
| hierarchy | 50/50, 0 vias, 1330 mm, 1 s, clean | **1 open**, 0 vias, 1377 mm, 7.8 s | complete, 18 vias, 1434 mm, 12 s, **10 bad** |
| pic | 34/34, 1 via, 1911 mm, 4 s, clean | **1 open**, 0 vias, 2101 mm, 7.2 s | complete, 16 vias, 2005 mm, 30 s, **180 bad** |
| interf-u | 110/110, **30 vias**, 4654 mm, 109 s, clean | complete, 44 vias, 5051 mm, 44 s, clean | **timed out** (25 min) |
| olimex-c3 | 34/34, 44 vias, 936 mm, 8 s, clean | **16 new hole-clearance errors** (DSN drops local clearances), 24 s | **61 open**, 101 vias, 84 s, 407 bad |
| dut-c3 | 42/42, 27 vias, 1295 mm, 7 s, clean | **2 open**, 23 vias, 1322 mm, 14 s | complete, 77 vias, 1377 mm, 27 s, **12 bad** |
| dut-c6 | 42/42, **26 vias**, 1274 mm, 22 s, clean | complete, 29 vias, 1355 mm, 14 s, clean | complete, 71 vias, 1301 mm, 10 s, clean |
| dut-s2 | 46/46, **34 vias**, 1492 mm, 17 s, clean | complete, 36 vias, 1582 mm, 13 s, clean | complete, 109 vias, 1548 mm, 20 s, clean |
| dut-s3 | 46/46, 33 vias, 1409 mm, 27 s, clean | complete, **31 vias**, 1472 mm, 11 s, clean | **1 open**, 108 vias, 1463 mm, 96 s, 8 bad |
| dut-esp32 | 46/46, **30 vias**, 1401 mm, 24 s, clean | complete, 35 vias, 1449 mm, 14 s, clean | complete, 90 vias, 1408 mm, 15 s, **1 bad** |
| sonde-xilinx | 26/26, 2 vias, 700 mm, 1 s, clean | complete, **0 vias**, 757 mm, 3.1 s, clean | **4 open**, 25 vias, 883 mm, 16 s, 52 bad |
| multichannel | 79/79, 20 vias, 2578 mm, 75 s, clean | **180 open** (via class below board minimum), 16 s | **81 open**, 130 vias, 20 s, 1194 bad |
| stickhub | 43/45, 53 vias, 752 mm, 13 s, **4 open** | **2 open**, 44 vias, 851 mm, 66 s | **226 open** (routing failed), 20 s |

Clean completions: pcb-maker 12 of 13, Freerouting 8 of 13, tscircuit 3 of 13
(dut-c6, dut-s2 and ecc83 apart from its 7 findings). Copper: pcb-maker
least on every board. Vias among the boards pcb-maker and Freerouting both
complete: pcb-maker fewer on four, Freerouting on two, equal on one; tscircuit
uses two to four times as many. Time: Freerouting is the fastest on the
mid-size boards; pcb-maker's via-reduction phase (three renegotiation rounds)
is what it spends its extra time on and can be switched off
(`via_reduction_rounds: 0`), which makes it 3-10x faster than either at
20-40 % more vias. StickHub remains the one board where Freerouting gets
further than pcb-maker.

tscircuit's numbers should be read with care: the bridge is new, the solver
has no trace-to-trace clearance field (spacing is encoded as width plus
clearance), and its own benchmarks are tscircuit-native boards rather than
KiCad projects. It is included because it is the other open-source router
being actively developed.

With pours kept (pcb-maker's normal route mode, which neither external router
can run) the same boards route with the same completion, fewer vias on most,
and 15-30 % less copper.

## Growing the corpus

The KiCad demo repository also has two very large boards (jetson-agx-thor, vme-wren: 1100-1500
footprints, 10-12 layers) as long-term targets. More open boards (Olimex and
others) can be added to `corpus.json`; boards whose source fails KiCad's checks
for other reasons than the baseline subtraction covers should be skipped.

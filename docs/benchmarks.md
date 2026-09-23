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

## Results (route column 2026-09-22 evening, layout column earlier that day; one thread)

Tracks and vias are stripped; copper pours stay (where copper is poured is a
design decision like the placement). `pours connect` means pads reached their
pour through stubs, vias and stitching; `pours connect+skeleton` adds the
fixed plane skeleton; `pours tracks` means the pour nets were routed as
tracks because connecting through the pour left something open (the
[attempt ladder](router.md#the-attempt-ladder) tries these in turn and keeps
the best board). Route times are the winning attempt's routing, with the
whole ladder's wall time including native checks in parentheses; place +
route times are the whole coupled loop including its routing probes and the
final native check. "Clean" means no unconnected items and no copper
DRC error beyond what the stripped source or the designer's own board has.

| Board | Reference copper | Route (designer placement) | Place + route (automatic) |
| --- | --- | --- | --- |
| ecc83 | 59 tracks, 0 vias, 1 zones | 9/9, 0 vias, 160 mm, 0.19 s (ladder 6 s), pours connect — clean | 9/9, 0 vias, 140 mm, 5.8 s, pours connect — clean |
| hierarchy | 364 tracks, 0 vias, 166 zones | 50/50, 0 vias, 1003 mm, 1.65 s (ladder 9 s), pours connect — clean | 50/50, 0 vias, 1119 mm, 10.0 s, pours connect — clean |
| pic | 370 tracks, 6 vias, 1 zones | 34/34, 2 vias, 1520 mm, 8.18 s (ladder 15 s), pours connect — clean | 34/34, 6 vias, 2100 mm, 23.0 s, pours connect — clean |
| interf-u | 731 tracks, 84 vias, 1 zones | 110/110, 28 vias, 4735 mm, 75 s (ladder 233 s), pours tracks — clean | 110/110, 63 vias, 4674 mm, 361.8 s, pours tracks — clean |
| olimex-c3 | 768 tracks, 88 vias, 160 zones | 34/34, 49 vias, 965 mm, 19 s (ladder 67 s), pours tracks — clean | 34/34, 61 vias, 912 mm, 28.8 s, pours tracks — clean |
| dut-c3 | 321 tracks, 57 vias, 2 zones | 42/42, 25 vias, 1102 mm, 23 s (ladder 30 s), pours connect — clean | 42/42, 23 vias, 956 mm, 17.3 s, pours connect — clean |
| dut-c6 | 359 tracks, 68 vias, 2 zones | 42/42, 32 vias, 1034 mm, 26 s (ladder 32 s), pours connect — clean | 42/42, 24 vias, 1020 mm, 14.4 s, pours connect — clean |
| dut-s2 | 432 tracks, 83 vias, 2 zones | 46/46, 44 vias, 1252 mm, 38 s (ladder 86 s), pours connect+skeleton — clean | 46/46, 51 vias, 1356 mm, 24.3 s, pours connect — clean |
| dut-s3 | 392 tracks, 64 vias, 2 zones | 46/46, 38 vias, 1147 mm, 39 s (ladder 45 s), pours connect — clean | 46/46, 42 vias, 1144 mm, 17.7 s, pours connect — clean |
| dut-esp32 | 345 tracks, 64 vias, 2 zones | 46/46, 41 vias, 1173 mm, 30 s (ladder 36 s), pours connect — clean | 46/46, 43 vias, 1446 mm, 29.7 s, pours tracks — clean |
| ngdevkit (designer never finished it; headers fixed by `corpus.json`) | 115 tracks, 29 vias, 2 zones | 153/180, 1060 vias, 18 730 mm, 1098 s (negotiation capped at 900 s), pours connect — **151 unconnected**; starved_thermal×41 | 161/180, 741 vias, 18 567 mm, 2566 s, pours tracks — **90 unconnected**; starved_thermal×8 |
| sonde-xilinx | 208 tracks, 3 vias, 1 zones | 26/26, 2 vias, 575 mm, 3.42 s (ladder 18 s), pours connect+skeleton — clean | 26/26, 1 vias, 789 mm, 15.3 s, pours tracks — clean |
| multichannel | 576 tracks, 29 vias, 2 zones | 79/79, 18 vias, 2090 mm, 40 s (ladder 47 s), pours connect — clean | 79/79, 39 vias, 2077 mm, 52.5 s, pours tracks — starved_thermal×1 |
| stickhub | 1113 tracks, 87 vias, 5 zones | 44/45, 56 vias, 572 mm, 25 s (ladder 198 s), pours connect — **1 unconnected**; solder_mask_bridge×8 | **placement not legal** |
| coldfire (4 layers, 209 nets) | 2935 tracks, 253 vias, 3 zones | 208/209, 198 vias, 9155 mm, 308 s (ladder 621 s), pours tracks — **11 unconnected** | 208/209, 207 vias, 8890 mm, 1189 s — 12 unconnected |
| video (4 layers, 371 nets, 189 footprints) | 7932 tracks, 808 vias, 2 zones | 362/371, 669 vias, 34180 mm, 936 s (ladder 950 s), pours connect — **45 unconnected**; starved_thermal×8 | timed out |
| openair-max (4 layers, 120 nets, 210 footprints) | 1638 tracks, 410 vias, 34 zones | 120/120, 114 vias, 4365 mm, 180 s (ladder 467 s), pours tracks — solder_mask_bridge×2 | **placement not legal** |
| tiny_tapeout (4 layers; its source fails KiCad DRC: 8 clearance errors, 3 shorts) | 2143 tracks, 405 vias, 2 zones | 78/108, 139 vias, 7053 mm, 189 s (ladder 583 s), pours tracks — **42 unconnected** | **placement not legal** |

The ngdevkit row is from 2026-09-23 (after the cutout, edge-stub and
unrouted-pad clearance fixes): no clearance or internal findings remain, only
the designer's own USB-C shell pads on the edge; completion varies between
runs (153–160 route, 161–173 layout) because the ladder's budgets are
wall-clock based.

Twelve of the thirteen two-layer boards are complete and clean in route
mode (StickHub is one pad short with its pours kept; on the cold board it
completes on the 0.075 mm rung), twelve in both modes. Of the four-layer
boards, openair-max (210 footprints) is complete, ColdFire is one net short,
video nine: video's single pour-connect attempt runs into the 900 s
negotiation cap and the ladder does not try the pour-as-tracks rung after an
attempt that slow (an earlier build reached 364/371 that way), and
tiny_tapeout's source fails KiCad's own checks.

## What the failures say

- **StickHub** (16 x 40 mm, two-sided SMD, 0.15 mm rules): on the regular
  0.1 mm lattice the 17-pin +3.3 V net does not fit once the other nets are
  in; the 0.075 mm rung completes the cold board (45/45, 47 vias, 41 s) and
  with the small +3.3 V pour regions kept the 0.05 mm rung gets to one open
  pad. The placer needs side assignment for a board this dense.
- **ngdevkit** (174 x 134 mm, 186 footprints, about 1070 pads, two layers,
  0.15 mm escape rules): two separate problems. First, its PSRAM is a 48-ball
  0.75 mm BGA whose inter-ball channels leave a 0.15 mm track 0.01 mm of
  slack, so the lattice must put a node row exactly mid-channel — a 0.1 mm
  lattice cannot, which made the inner balls unreachable and every net to
  them a permanent failure; the lattice choice now scores such tight channels
  and picks 0.125 mm (see [router.md](router.md)). Second, with the balls
  reachable, the negotiation still does not converge: 50-90 nets stay in
  conflict at the present-cost cap, and the congestion history piles up on
  the front layer at the BGA (x 145-151, y 78-84 mm), where the 16-bit data
  and 20-bit address buses from the level translators, the expander bus and
  the BGA escape all meet. The ground pour (224 pads on both layers) is
  shredded into 1500 pieces on the way; a plane skeleton keeps its pads
  connected but does not make the signals fit. As placed by its (unfinished)
  designer the board is most likely not routable on two layers; it is the
  test case for automatic re-placement, not for the router alone. With the
  53 headers and the ESP32 module held (the headers are the ribbon
  interface; the module sits at its antenna keepout) and everything else
  re-placed, the coupled loop gets to 173/180 in 43 minutes — better than
  the designer's placement, still not a board.
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
| multichannel | 79/79, 20 vias, 2578 mm, 75 s, clean | **180 open** (via class below board minimum; suspected setup problem, see [todo](todo.md)), 16 s | **81 open**, 130 vias, 20 s, 1194 bad |
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

## Strength: the same design on a smaller board

`run.py --shrink` (default factors 0.9, 0.8, 0.7, 0.6, 0.5) scales a board's
outline and every position on it towards the centre by a factor, keeps part
sizes, pushes parts back inside the outline where they would stick out, and
places and routes the result from scratch (`pcb-maker shrink-kicad-board`
does the transformation alone). Only the designer's own findings are
forgiven. The table's "smallest clean board" column is the last factor that
routed clean, with what stopped the next one: the strength of the
place-and-route flow in one number per board. The limit is often physical
before it is algorithmic — dut-c3 at 0.8 puts a 50 mm header on a 52 mm
board, into the corner mounting holes. Per-board placement constraints
(`"layout"` in `corpus.json`, e.g. ngdevkit's headers) apply to the shrunk
boards as well.

## Growing the corpus

The KiCad demo repository also has two very large boards (jetson-agx-thor, vme-wren: 1100-1500
footprints, 10-12 layers) as long-term targets. More open boards (Olimex and
others) can be added to `corpus.json`; boards whose source fails KiCad's checks
for other reasons than the baseline subtraction covers should be skipped.

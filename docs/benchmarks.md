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

## Nightly

Main measured every night by `scripts/nightly.sh`
([benchmarks/agent-tasks/README.md](../benchmarks/agent-tasks/README.md#nightly),
"Nightly"): the fingerprints (Castor, k30-SBC, jetson, Sisu in route mode
from v16), the route-mode expansion tier, the short tier and, on the
configured days, the full layout sweep (tasks v7, `--quality`) with the
matched designer tier. One row a night, newest first, the last 14 nights,
written by `scripts/nightly-report.py` from `build/nightly/<night>/`
(`summary.md` there has the per-board detail). Cells: passes / rows
(strict passes), KiCad's unconnected items, rows cut by the wall clock
and killed; the designer tier's budget scale; the gap split over the
target boards (`compare.py --gap --targets`); against the previous
nightly that ran each stage, better / same / worse (compare.py) and the
boards that gained or lost the pass.

<!-- nightly:begin -->
| Night | main | Fingerprints | Expansion | Short | Sweep | Designer | Gap (placement / routing) | Against the previous night (better/same/worse) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
<!-- nightly:end -->

## Fence (2026-10-21)

Every breadboard-fence benchmark run once with x1404 (main at the fence
report's router side) and, where this pass changed something, with
x1462 (branch fence-benchmark): the table is in
[benchmarks/fence/README.md](../benchmarks/fence/README.md#results)
("benchmark pass" rows), with vias in pads by pcbnew's HitTest (0 on
every board) and the GND pour by KiCad's fill (`pour-pieces.py`) and by
the router's pour model (the verdict's new `pours`). In short: v5 four
layers complete and clean; v5 two layers at 34 mm 5 unconnected; at 46 mm
every connection with one starved thermal, and complete and clean with
a `keep_whole` strip that grew the pour's largest piece from 293 to 373
mm² (at 44 mm, the free dimension's size: 389 to 630 mm², clean both); v1 two layers 100/100 for the first time (one starved thermal);
the corpus's v4 four-layer boards complete and clean in both modes.

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
  pad. Since 2026-09-29 automatic placement is legal (bodies closer to
  the edge as their copper allows, legalization retried with the stuck
  parts first). Placed and routed it reaches 41/45, with solder-mask
  bridges as in route mode.
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
- **Automatic placement on dense two-sided boards.** Since 2026-09-29
  StickHub and openair-max place legally. tiny_tapeout still leaves one
  connector (J4) without a place, with its front 78 % full.

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

## Layout sweep v11 on the harvested boards (2026-10-09)

109 boards, each reduced to what an agent starts from (parts stacked, the
designer's placement gone, a `constraints.json` derived from it: fixed
connectors, edges), laid out with `pcb-maker-x136` under 1800 s per board,
three at a time; boards killed or panicked rerun on x137-x146. A pass is
KiCad's verdict: 0 unconnected, no copper error beyond the designer's own
board, constraints met. **38 of 109 pass.** The table compares with the
x112 baseline (29 boards both have; before / after as routed, KiCad
unconnected, `+Ne` copper errors, seconds):

    benchmarks/agent-tasks/compare.py build/github-layout-v8-x112.log build/github-layout-v11.log --splice <reruns> --markdown

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
vd-rd__sbc_allwinner_a13__module                                     |              173/223 125 |           216/223 26 +1e |   1538
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |          160/192 108 +2e |               185/192 26 |   1638
OpenDrone-hw__OpenESC-30x30__4in1                                    |               87/152 146 |               120/152 84 |    969
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               109/152 97 |               122/152 51 |    802
oro-os__link__link                                                   |              224/251 105 |               230/251 70 |   1662
CRImier__MyKiCad__vaio_re                                            |               256/267 31 |                262/267 5 |   1407
thpoll83__PolyKybd__poly_corne_split42_right                         |               419/420 16 |            419/420 9 +2e |   1662
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                337/340 8 |                338/340 2 |   1525
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               151/186 86 |               151/186 82 |    925
earth75__atat-1800__ATAT1800                                         |           249/278 44 +7e |           254/278 40 +5e |   1695
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           144/159 20 +4e |           146/159 17 +3e |   1372
emertcakir__OpenAirScope__OpenAirScope                               |                176/184 8 |                179/184 5 |   1185
thpoll83__PolyKybd__poly_kybd_split72_right                          |               416/420 21 |               419/420 18 |   1665
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                131/133 2 |                132/133 1 |    889
CRImier__MyKiCad__framework_mobo_lefthalf                            |             71/71 0 pass |             71/71 0 pass |    215
CRImier__MyKiCad__zpn_devboard                                       |           167/167 0 pass |           167/167 0 pass |    470
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |            138/138 0 +1e |            138/138 0 +1e |    248
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                141/144 5 |                141/144 5 |    789
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    142
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    329
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               43/184 499 |               32/184 499 |    701
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  80/82 2 |    671
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                  55/56 4 |    228
byrantech__laptop__motherboard                                       |                234/236 2 |                234/236 2 |   1434
tengigabytes__MokyaLora__MokyaLora                                   |           261/262 0 pass |           261/262 0 pass |   1196
thpoll83__PolyKybd__poly_corne_split42_left                          |                245/247 3 |                245/247 3 |   1546
thpoll83__PolyKybd__poly_kybd_split72_left                           |                417/420 6 |                417/420 6 |   1662
hackclub__OnBoard__krishveercard                                     |                 28/43 20 |                 23/43 21 |    830
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               53/127 240 |               39/127 253 |    719
0xCB-dev__0xCB-1337__1337-v4.0                                       |                        - |             47/47 0 pass |     90
0xCB-dev__0xCB-1337__panel                                           |                        - |           188/188 0 pass |    533
0xCB-dev__0xCB-1337__pcb                                             |                        - |             85/85 0 pass |    207
0xCB-dev__0xCB-1337__pcb-panel                                       |                        - | ERROR (error: kicad-cli timed out aft) |    993
0xCB-dev__0xCB-Static__0xcb-static                                   |                        - |             74/74 0 pass |     25
ASH-ART__Qfwfq__qfwfq                                                |                        - |             64/64 0 pass |    205
CDFER__Business-Cards__Batch_1                                       |                        - |              124/204 468 |   1077
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |                        - |              54/54 0 +1e |    242
CDFER__Business-Cards__USB_Keypad                                    |                        - |             39/39 0 pass |    117
CDFER__Business-Cards__WLED_Matrix                                   |                        - |                101/104 1 |    380
CRImier__MyKiCad__protoesp                                           |                        - |            93/104 21 +1e |    936
GlasgowEmbedded__glasgow__glasgow__revC3                             |                        - |               177/226 90 |   1144
Goga64__ULK__ULK                                                     |                        - |              70/70 0 +1e |    514
Goga64__ULK__ULK_sl_PG1316s                                          |                        - |              70/70 0 +3e |    338
Huaqiu-Electronics__ecad-viewer__video                               |                        - |           371/371 0 pass |   1012
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |                        - |                151/156 8 |    584
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |                        - |                154/159 8 |    556
Jana-Marie__ligra__ligra_back                                        |                        - |             33/33 0 pass |     41
Ladniy__jiran-ble-lite__jiran-ble-lite                               |                        - |               104/105 20 |    480
Neotron-Compute__Neotron-Pico__neotron-pico                          |                        - |           176/190 24 +2e |   1186
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |                        - |             21/21 0 pass |     98
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |                        - |             19/19 0 pass |    109
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |                        - |           108/108 0 pass |    660
anyshake__explorer__Explorer                                         |                        - |               140/150 20 |   1140
baldengineer__bit-preserve__coco2                                    |                        - |           173/173 0 +12e |    442
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |                        - |                 57/69 14 |    550
bitshiftcrazy__d20_pcb__d20_pcb                                      |                        - |             1/19 63 +27e |   1398
bitshiftcrazy__spell_tome__spell_tome_bottom                         |                        - |             18/18 0 pass |     29
byrantech__laptop__keyboard                                          |                        - |                132/136 3 |    784
byrantech__laptop__power                                             |                        - |           115/115 0 pass |    210
crmaykish__mackerel-68k__mackerel-08-v1                              |                        - |              96/96 0 +5e |    281
crmaykish__mackerel-68k__mackerel-10-v1                              |                        - |           155/155 0 pass |    911
crmaykish__mackerel-68k__mackerel-30-proto                           |                        - |           217/217 0 +15e |    613
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |                        - |           585/585 0 pass |     81
duckyb__eternal-keypad__eternal-keypad                               |                        - |              69/69 0 +2e |    237
duckyb__urchin__main                                                 |                        - |             68/68 0 pass |     13
ebastler__osprey__osprey_rev_a                                       |                        - |                101/103 1 |    304
greatscottgadgets__hackrf__hackrf-one                                |                        - |            318/319 1 +1e |    780
hackclub__OnBoard__E-Fidget-Lite                                     |                        - |             20/20 0 pass |     14
hackclub__OnBoard__MotionCubeViewAllForces                           |                        - |             20/20 0 pass |    224
hackclub__OnBoard__PixelWave                                         |                        - |            343/343 0 +1e |    328
hackclub__OnBoard__keyboar_                                          |                        - |           102/102 0 pass |    268
hackclub__OnBoard__koeg-board-pcb                                    |                        - |           170/175 6 +15e |   1377
hackclub__OnBoard__woagboard                                         |                        - |                  90/91 2 |    850
headblockhead__slab-pcb__interchange-pcb-right                       |                        - |              83/83 8 +1e |   1016
headblockhead__slab-pcb__slab-pcb-left                               |                        - |             76/76 0 pass |    488
iandchasse__silkscreen-pcb__silkscreen_pcb                           |                        - |            113/113 1 +1e |    302
ikajdan__katia__katia                                                |                        - | ERROR (note: run with `RUST_BACKTRACE) |   1762
little-red-rover__little-red-rover__little_red_rover                 |                        - |             64/64 0 pass |    771
maniekx86__M8SBC-486__homebrew_486                                   |                        - |           190/190 0 pass |   1233
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |                        - |          169/172 11 +29e |   1536
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |                        - |            162/162 0 +4e |    434
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |                        - |            189/190 5 +1e |    841
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |                        - |            148/148 0 +6e |   1120
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |                        - |              58/58 0 +2e |     76
sporkus__capybully_keyboard__capybully                               |                        - |                  32/32 1 |   1383
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |                        - |             76/76 0 pass |    190
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |                        - |             75/75 0 pass |    298
stonedDiscord__MegaDrive__MegaDrive                                  |                        - |              274/298 107 |   1581
stonedDiscord__nonSNES__SNSP-CPU-01                                  |                        - |         333/363 116 +58e |   1666
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |                        - |           206/212 23 +5e |   1461
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |                        - |            8/28 499 +11e |   1261
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |                        - |            5/28 499 +11e |   1279
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |                        - |            4/28 499 +10e |   1035
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |                        - |            5/28 499 +18e |   1293
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |                        - |            5/28 499 +19e |   1309
tomunderwood99__CharlieBoard__Blue_Line                              |                        - |             26/26 0 pass |     35
transistorfet__computie__k30-SBC                                     |                        - |           173/173 0 +10e |    594
tubbytwins__bumwings-kbd__bumwings_v001                              |                        - |             72/72 0 pass |     78
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |                        - |             70/70 0 pass |     51
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |                        - |             75/75 0 pass |    178
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |                        - |             85/85 0 pass |    220
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |                        - |             81/81 0 pass |     45
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |                        - |             89/89 0 pass |     73
tubbytwins__bumwings-kbd__bumwings_v001_core                         |                        - |              98/98 0 +4e |    262
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |                        - |             81/81 0 pass |    393
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |                        - |              78/78 0 +2e |    220
tzarc__keyboards__ghoul                                              |                        - |                142/148 6 |    821
wntrblm__Castor_and_Pollux__mainboard                                |                        - |           126/126 0 +17e |    282
zli117__CyberKeeb2040__MainBoard                                     |                        - |           118/118 0 pass |    144

29 boards in both: better 14, same 13, worse 2; unconnected 1598 -> 1231; passes 5 -> 5
80 boards only after: 33 pass

Not targets among the boards only the after-run has: the five PolyKybd
molecule panels (duplicated references, the designer's board has 266
unconnected), the OpenESC and OpenRX panels, krishveercard and SUMEC (rules
no track of their class can satisfy).

## Layout sweep v14 (2026-10-10): the 16-seed race, the placer's fixes, tasks v6

`pcb-maker-x510` (aa4e9e8: the RUDY-ranked 16-seed race, the placer's
correctness rounds, the corridor schedule and the stall stop) on the v6
tasks (overhang tested against the outline polygon: 35 tasks gain fixed
parts), 10 jobs, 12668 s of wall clock: **34 of 109 pass** (v13: 31);
against v13 better on 19, same on 51, worse on 24, 3151 to 3065 open in
all (tasks differ on 35 boards, so this is a new baseline rather than a
clean A/B). Error rows: the five molecule panels and the OpenESC 4in1
panel find no legal placement; link, katia, MegaDrive and the 0xCB panel
hit the 1800 s limit (PixelWave and SmartSpin2k no longer do).

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
Huaqiu-Electronics__ecad-viewer__video                               |              140/371 499 |              253/371 315 |   1671
CRImier__MyKiCad__vaio_re                                            |          182/267 232 +2e |           240/267 81 +1e |   1270
vd-rd__sbc_allwinner_a13__module                                     |           176/223 97 +1e |            221/223 8 +1e |   1122
hackclub__OnBoard__koeg-board-pcb                                    |          156/175 53 +16e |          166/175 11 +15e |   1256
CRImier__MyKiCad__zpn_devboard                                       |          117/167 107 +1e |               128/167 71 |    854
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |           183/192 42 +2e |           188/192 18 +1e |   1529
CDFER__Business-Cards__WLED_Matrix                                   |           88/104 28 +72e |             99/104 9 +8e |    831
hackclub__OnBoard__woagboard                                         |            75/91 27 +55e |            85/91 11 +34e |   1706
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |               118/133 38 |               116/133 27 |    892
Ladniy__jiran-ble-lite__jiran-ble-lite                               |                98/105 28 |           104/105 20 +2e |    562
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |                 65/69 35 |                 66/69 28 |    923
Goga64__ULK__ULK                                                     |              65/70 8 +2e |                  68/70 2 |   1742
byrantech__laptop__motherboard                                       |                233/236 7 |                233/236 3 |   1693
maniekx86__M8SBC-486__homebrew_486                                   |                189/190 3 |           190/190 0 pass |   1145
anyshake__explorer__Explorer                                         |            147/150 3 +2e |                149/150 1 |    596
duckyb__urchin__main                                                 |                  66/68 2 |             68/68 0 pass |     54
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           139/159 33 +1e |               139/159 32 |   1099
ebastler__osprey__osprey_rev_a                                       |            101/103 2 +4e |            102/103 1 +3e |    374
sporkus__capybully_keyboard__capybully                               |                  32/32 3 |                  32/32 2 |   1286
0xCB-dev__0xCB-1337__1337-v4.0                                       |             47/47 0 pass |             47/47 0 pass |    166
0xCB-dev__0xCB-1337__pcb                                             |             85/85 0 pass |             85/85 0 pass |    176
0xCB-dev__0xCB-Static__0xcb-static                                   |             74/74 0 pass |             74/74 0 pass |    154
ASH-ART__Qfwfq__qfwfq                                                |             64/64 0 pass |             64/64 0 pass |    297
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              54/54 0 +1e |             54/54 0 pass |    252
CDFER__Business-Cards__USB_Keypad                                    |                  37/39 2 |                  37/39 2 |    411
Goga64__ULK__ULK_sl_PG1316s                                          |                  68/70 2 |                  68/70 2 |   1678
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |            138/138 0 +2e |           138/138 0 pass |    836
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |           144/144 0 pass |           144/144 0 pass |    778
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    455
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    418
Jana-Marie__ligra__ligra_back                                        |             33/33 0 pass |             33/33 0 pass |    110
Neotron-Compute__Neotron-Pico__neotron-pico                          |               176/190 24 |           176/190 24 +1e |   1001
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |             21/21 0 pass |             21/21 0 pass |    183
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |             19/19 0 pass |             19/19 0 pass |    301
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  81/82 2 |    727
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |            108/108 0 +3e |            108/108 0 +4e |    730
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |            332/340 9 +3e |           333/340 9 +11e |   1677
baldengineer__bit-preserve__coco2                                    |           173/173 0 pass |           173/173 0 pass |    788
bitshiftcrazy__d20_pcb__d20_pcb                                      |                  18/19 3 |                  18/19 3 |    831
bitshiftcrazy__spell_tome__spell_tome_bottom                         |             18/18 0 pass |              18/18 0 +1e |    112
byrantech__laptop__keyboard                                          |            132/136 3 +6e |                132/136 3 |    552
crmaykish__mackerel-68k__mackerel-08-v1                              |              96/96 0 +6e |              96/96 0 +3e |    476
crmaykish__mackerel-68k__mackerel-10-v1                              |           155/155 0 pass |           155/155 0 pass |    870
crmaykish__mackerel-68k__mackerel-30-proto                           |            217/217 0 +7e |            217/217 0 +5e |   1088
duckyb__eternal-keypad__eternal-keypad                               |             69/69 0 pass |              69/69 0 +1e |    266
hackclub__OnBoard__E-Fidget-Lite                                     |             20/20 0 pass |             20/20 0 pass |     62
hackclub__OnBoard__MotionCubeViewAllForces                           |             20/20 0 pass |             20/20 0 pass |    181
hackclub__OnBoard__keyboar_                                          |            102/102 0 +1e |           102/102 0 pass |    419
headblockhead__slab-pcb__slab-pcb-left                               |             76/76 0 pass |             76/76 0 pass |    533
iandchasse__silkscreen-pcb__silkscreen_pcb                           |            113/113 1 +1e |            113/113 1 +2e |    752
little-red-rover__little-red-rover__little_red_rover                 |             64/64 0 pass |             64/64 0 pass |    236
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |           162/162 0 +49e |           162/162 0 +47e |   1130
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |            148/148 0 +4e |            148/148 0 +4e |    731
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |             58/58 0 pass |             58/58 0 pass |    244
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |             76/76 0 pass |             76/76 0 pass |    440
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |             75/75 0 pass |             75/75 0 pass |    381
tengigabytes__MokyaLora__MokyaLora                                   |            261/262 0 +3e |            261/262 0 +1e |   1284
tomunderwood99__CharlieBoard__Blue_Line                              |             26/26 0 pass |             26/26 0 pass |     24
transistorfet__computie__k30-SBC                                     |           173/173 0 +10e |           173/173 0 +15e |    617
tubbytwins__bumwings-kbd__bumwings_v001                              |             72/72 0 pass |             72/72 0 pass |    121
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |             70/70 0 pass |             70/70 0 pass |    129
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |             75/75 0 pass |             75/75 0 pass |    180
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |              85/85 0 +1e |             85/85 0 pass |    271
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |              81/81 0 +1e |              81/81 0 +1e |    149
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |             89/89 0 pass |             89/89 0 pass |    268
tubbytwins__bumwings-kbd__bumwings_v001_core                         |             98/98 0 pass |             98/98 0 pass |    454
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |             81/81 0 pass |             81/81 0 pass |    415
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |             78/78 0 pass |              78/78 0 +1e |    242
wntrblm__Castor_and_Pollux__mainboard                                |           126/126 0 +14e |            126/126 0 +6e |    716
zli117__CyberKeeb2040__MainBoard                                     |           118/118 0 pass |           118/118 0 pass |    645
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |               151/156 11 |               150/156 12 |    844
byrantech__laptop__power                                             |                113/115 2 |                112/115 3 |   1099
greatscottgadgets__hackrf__hackrf-one                                |           319/319 0 +20e |            318/319 1 +6e |   1106
hackclub__OnBoard__krishveercard                                     |                 30/43 13 |                 34/43 14 |    769
headblockhead__slab-pcb__interchange-pcb-right                       |                  83/83 7 |                  83/83 8 |   1004
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |           212/212 0 pass |           210/212 2 +16e |    735
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |               152/159 12 |               151/159 16 |    836
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                 53/56 10 |    387
emertcakir__OpenAirScope__OpenAirScope                               |            184/184 0 +1e |                178/184 6 |   1003
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               50/127 249 |               44/127 257 |   1280
thpoll83__PolyKybd__poly_corne_split42_left                          |               225/247 33 |               218/247 43 |   1338
CRImier__MyKiCad__protoesp                                           |            87/104 55 +3e |            82/104 66 +5e |    977
earth75__atat-1800__ATAT1800                                         |           255/278 36 +1e |           242/278 49 +7e |   1409
thpoll83__PolyKybd__poly_kybd_split72_right                          |               399/420 44 |               389/420 58 |   1677
stonedDiscord__nonSNES__SNSP-CPU-01                                  |              318/363 224 |         302/363 239 +22e |   1481
GlasgowEmbedded__glasgow__glasgow__revC3                             |              111/226 237 |              113/226 254 |   1139
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               149/186 81 |              148/186 100 |   1052
thpoll83__PolyKybd__poly_corne_split42_right                         |               404/420 36 |           393/420 57 +1e |   1716
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |          145/172 96 +68e |         146/172 128 +70e |   1671
thpoll83__PolyKybd__poly_kybd_split72_left                           |               409/420 23 |               394/420 55 |   1670
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               128/152 59 |                95/152 96 |   1401
CDFER__Business-Cards__Batch_1                                       |              118/204 447 |              102/204 499 |   1787
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |          168/190 65 +14e |         118/190 167 +49e |   1010
OpenDrone-hw__OpenESC-30x30__4in1                                    |               89/152 124 |               61/152 239 |   1663
0xCB-dev__0xCB-1337__panel                                           | ERROR (timed out after 1800 s) |          145/188 129 +2e |   1660
0xCB-dev__0xCB-1337__pcb-panel                                       | ERROR (timed out after 1800 s) | ERROR (timed out after 1800 s) |   1800
CRImier__MyKiCad__framework_mobo_lefthalf                            | ERROR (error: placement is not legal ) |             71/71 0 pass |    450
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               42/184 499 | ERROR (error: placement is not legal ) |   1204
doudar__SmartSpin2k__SmartSpin2k_Panelized                           | ERROR (timed out after 1800 s) |              278/585 499 |   1742
hackclub__OnBoard__PixelWave                                         | ERROR (timed out after 1800 s) |           343/343 0 +16e |   1419
ikajdan__katia__katia                                                | ERROR (timed out after 1800 s) | ERROR (timed out after 1800 s) |   1801
oro-os__link__link                                                   |               242/251 15 | ERROR (timed out after 1800 s) |   1802
stonedDiscord__MegaDrive__MegaDrive                                  | ERROR (timed out after 1800 s) | ERROR (timed out after 1800 s) |   1800
thpoll83__PolyKybd__poly_kb_molecule_4x4                             | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |   1345
thpoll83__PolyKybd__poly_kb_molecule_4x5                             | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |   1659
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |    825
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |    947
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   | ERROR (error: placement is not legal ) | ERROR (error: placement is not legal ) |    814
tzarc__keyboards__ghoul                                              | ERROR (error: placement is not legal ) |                142/148 7 |   1474

109 boards in both: better 19, same 51, worse 24; unconnected 3151 -> 3065; passes 31 -> 34

## Layout sweep v13 (2026-10-10): every board placed from stacked parts

The first sweep after the task-preparation fix (`run.py`'s unplace now
reads pre-KiCad-10 net syntax, so all 109 boards lose the designer's
placement, not 22), on `pcb-maker-x503` (3b890fa: the pour model, the
parallel ladder, work-based and deterministic layout decisions, the
placer's correctness fixes; the 16-seed RUDY race came one commit later),
12 jobs, load 25-40, 14525 s of wall clock: **31 of 109 pass**, 3586
connections open in all against v12's 2151 on the same boards (v12 kept
the designer's placement on 80 of them). Six boards gained a pass through
the fixes (BAGEL-MK1.1, SNSP-CPU-1CHIP, two bumwings, coco2, Qfwfq); 16
lost theirs to real placement (MokyaLora, zpn, video, homebrew 486,
hackrf, laptop power, ...). Error rows: 7 boards where the placer finds
no legal placement (the five molecule panels, framework_mobo's M1,
ghoul's switches), 6 at the 1800 s limit (katia, PixelWave, SmartSpin2k,
MegaDrive, the two 0xCB panels).

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
bitshiftcrazy__d20_pcb__d20_pcb                                      |             1/19 63 +27e |                  18/19 3 |   1161
oro-os__link__link                                                   |               232/251 56 |               242/251 15 |   1748
hackclub__OnBoard__krishveercard                                     |                 23/43 21 |                 30/43 13 |   1182
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                141/144 5 |           144/144 0 pass |    683
emertcakir__OpenAirScope__OpenAirScope                               |                179/184 5 |            184/184 0 +1e |   1648
anyshake__explorer__Explorer                                         |            147/150 7 +1e |            147/150 3 +2e |   1024
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               150/186 84 |               149/186 81 |   1155
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |                210/212 3 |           212/212 0 pass |   1441
sporkus__capybully_keyboard__capybully                               |                  32/32 5 |                  32/32 3 |   1699
0xCB-dev__0xCB-1337__1337-v4.0                                       |             47/47 0 pass |             47/47 0 pass |    272
0xCB-dev__0xCB-1337__pcb                                             |             85/85 0 pass |             85/85 0 pass |    301
0xCB-dev__0xCB-Static__0xcb-static                                   |             74/74 0 pass |             74/74 0 pass |    205
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              54/54 0 +1e |              54/54 0 +1e |    220
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |           138/138 0 pass |            138/138 0 +2e |    800
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    736
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    753
Jana-Marie__ligra__ligra_back                                        |             33/33 0 pass |             33/33 0 pass |    124
Neotron-Compute__Neotron-Pico__neotron-pico                          |           176/190 24 +1e |               176/190 24 |   1403
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |             21/21 0 pass |             21/21 0 pass |    214
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |             19/19 0 pass |             19/19 0 pass |    596
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               128/152 59 |               128/152 59 |    976
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               32/184 499 |               42/184 499 |   1524
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  81/82 2 |    690
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                  55/56 4 |    553
bitshiftcrazy__spell_tome__spell_tome_bottom                         |             18/18 0 pass |             18/18 0 pass |     78
byrantech__laptop__keyboard                                          |                132/136 3 |            132/136 3 +6e |   1131
crmaykish__mackerel-68k__mackerel-08-v1                              |              96/96 0 +3e |              96/96 0 +6e |   1086
crmaykish__mackerel-68k__mackerel-10-v1                              |           155/155 0 pass |           155/155 0 pass |    876
crmaykish__mackerel-68k__mackerel-30-proto                           |            217/217 0 +6e |            217/217 0 +7e |   1659
duckyb__eternal-keypad__eternal-keypad                               |             69/69 0 pass |             69/69 0 pass |    324
earth75__atat-1800__ATAT1800                                         |          251/278 36 +18e |           255/278 36 +1e |   1715
greatscottgadgets__hackrf__hackrf-one                                |           319/319 0 pass |           319/319 0 +20e |   1272
hackclub__OnBoard__E-Fidget-Lite                                     |             20/20 0 pass |             20/20 0 pass |    123
hackclub__OnBoard__MotionCubeViewAllForces                           |             20/20 0 pass |             20/20 0 pass |    213
hackclub__OnBoard__keyboar_                                          |           102/102 0 pass |            102/102 0 +1e |    832
headblockhead__slab-pcb__interchange-pcb-right                       |              83/83 7 +2e |                  83/83 7 |   1336
headblockhead__slab-pcb__slab-pcb-left                               |             76/76 0 pass |             76/76 0 pass |   1217
iandchasse__silkscreen-pcb__silkscreen_pcb                           |            113/113 1 +1e |            113/113 1 +1e |   1419
little-red-rover__little-red-rover__little_red_rover                 |             64/64 0 pass |             64/64 0 pass |    554
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |            162/162 0 +2e |           162/162 0 +49e |    964
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |            148/148 0 +3e |            148/148 0 +4e |    979
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |             58/58 0 pass |             58/58 0 pass |    512
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |             76/76 0 pass |             76/76 0 pass |    304
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |             75/75 0 pass |             75/75 0 pass |    240
tengigabytes__MokyaLora__MokyaLora                                   |           261/262 0 pass |            261/262 0 +3e |   1325
tomunderwood99__CharlieBoard__Blue_Line                              |             26/26 0 pass |             26/26 0 pass |    114
transistorfet__computie__k30-SBC                                     |           173/173 0 +11e |           173/173 0 +10e |   1032
tubbytwins__bumwings-kbd__bumwings_v001                              |             72/72 0 pass |             72/72 0 pass |    189
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |             70/70 0 pass |             70/70 0 pass |    281
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |             75/75 0 pass |             75/75 0 pass |    205
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |             85/85 0 pass |              85/85 0 +1e |    221
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |             81/81 0 pass |              81/81 0 +1e |    158
tubbytwins__bumwings-kbd__bumwings_v001_core                         |             98/98 0 pass |             98/98 0 pass |    815
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |             81/81 0 pass |             81/81 0 pass |    867
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |              78/78 0 +1e |             78/78 0 pass |    456
wntrblm__Castor_and_Pollux__mainboard                                |            126/126 0 +2e |           126/126 0 +14e |    990
zli117__CyberKeeb2040__MainBoard                                     |           118/118 0 pass |           118/118 0 pass |    671
ebastler__osprey__osprey_rev_a                                       |                101/103 1 |            101/103 2 +4e |    384
CDFER__Business-Cards__USB_Keypad                                    |             39/39 0 pass |                  37/39 2 |    531
Goga64__ULK__ULK_sl_PG1316s                                          |              70/70 0 +3e |                  68/70 2 |   1687
byrantech__laptop__power                                             |           115/115 0 pass |                113/115 2 |   1363
duckyb__urchin__main                                                 |             68/68 0 pass |                  66/68 2 |    339
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |                151/156 8 |               151/156 11 |    696
maniekx86__M8SBC-486__homebrew_486                                   |           190/190 0 pass |                189/190 3 |   1667
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |                154/159 8 |               152/159 12 |   1216
byrantech__laptop__motherboard                                       |                234/236 2 |                233/236 7 |   1675
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                338/340 2 |            332/340 9 +3e |   1675
Goga64__ULK__ULK                                                     |              70/70 0 +1e |              65/70 8 +2e |   1680
Ladniy__jiran-ble-lite__jiran-ble-lite                               |               104/105 20 |                98/105 28 |    848
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               54/127 237 |               50/127 249 |   1082
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           146/159 17 +2e |           139/159 33 +1e |   1313
stonedDiscord__nonSNES__SNSP-CPU-01                                  |              308/363 206 |              318/363 224 |   1720
thpoll83__PolyKybd__poly_kybd_split72_left                           |                418/420 3 |               409/420 23 |   1719
hackclub__OnBoard__woagboard                                         |                  90/91 2 |            75/91 27 +55e |   1667
CDFER__Business-Cards__WLED_Matrix                                   |                101/104 1 |           88/104 28 +72e |    486
thpoll83__PolyKybd__poly_corne_split42_left                          |                246/247 4 |               225/247 33 |   1681
vd-rd__sbc_allwinner_a13__module                                     |           181/223 68 +1e |           176/223 97 +1e |   1146
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |               183/192 12 |           183/192 42 +2e |   1408
thpoll83__PolyKybd__poly_corne_split42_right                         |                418/420 4 |               404/420 36 |   1754
CDFER__Business-Cards__Batch_1                                       |              125/204 414 |              118/204 447 |   1547
CRImier__MyKiCad__protoesp                                           |                93/104 18 |            87/104 55 +3e |    956
OpenDrone-hw__OpenESC-30x30__4in1                                    |               118/152 87 |               89/152 124 |   1316
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                132/133 1 |               118/133 38 |    926
hackclub__OnBoard__koeg-board-pcb                                    |           168/175 9 +15e |          156/175 53 +16e |   1683
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |          147/172 36 +26e |          145/172 96 +68e |   1687
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |                189/190 5 |          168/190 65 +14e |   1357
CRImier__MyKiCad__zpn_devboard                                       |           167/167 0 pass |          117/167 107 +1e |   1150
GlasgowEmbedded__glasgow__glasgow__revC3                             |               173/226 93 |              111/226 237 |   1572
CRImier__MyKiCad__vaio_re                                            |                260/267 9 |          182/267 232 +2e |   1670
Huaqiu-Electronics__ecad-viewer__video                               |           371/371 0 pass |              140/371 499 |   1728
0xCB-dev__0xCB-1337__panel                                           |           188/188 0 pass | ERROR (timed out after 1800 s) |   1801
0xCB-dev__0xCB-1337__pcb-panel                                       | ERROR (error: kicad-cli timed out aft) | ERROR (timed out after 1800 s) |   1800
ASH-ART__Qfwfq__qfwfq                                                | ERROR (note: run with `RUST_BACKTRACE) |             64/64 0 pass |    637
CRImier__MyKiCad__framework_mobo_lefthalf                            |             71/71 0 pass | ERROR (error: placement is not legal ) |     13
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        | ERROR (error: rule area polygon has f) |            108/108 0 +3e |    989
baldengineer__bit-preserve__coco2                                    | ERROR (note: run with `RUST_BACKTRACE) |           173/173 0 pass |   1335
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              | ERROR (note: run with `RUST_BACKTRACE) |                 65/69 35 |   1271
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |           585/585 0 pass | ERROR (timed out after 1800 s) |   1800
hackclub__OnBoard__PixelWave                                         |           343/343 0 pass | ERROR (timed out after 1800 s) |   1800
ikajdan__katia__katia                                                |               148/162 99 | ERROR (timed out after 1800 s) |   1801
stonedDiscord__MegaDrive__MegaDrive                                  |              271/298 119 | ERROR (timed out after 1800 s) |   1801
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |            4/28 499 +12e | ERROR (error: placement is not legal ) |    352
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |            4/28 499 +20e | ERROR (error: placement is not legal ) |    646
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |             4/28 499 +7e | ERROR (error: placement is not legal ) |    426
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |            5/28 499 +34e | ERROR (error: placement is not legal ) |    328
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |            3/28 499 +29e | ERROR (error: placement is not legal ) |    661
thpoll83__PolyKybd__poly_kybd_split72_right                          | ERROR (timed out after 1800 s) |               399/420 44 |   1725
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   | ERROR (note: run with `RUST_BACKTRACE) |             89/89 0 pass |    316
tzarc__keyboards__ghoul                                              |                142/148 6 | ERROR (error: placement is not legal ) |    944

109 boards in both: better 9, same 48, worse 33; unconnected 2151 -> 3586; passes 41 -> 31

## Layout sweep v12 (2026-10-09, later): the pour model and the parallel ladder

The same 109 tasks on `pcb-maker-x165` (HEAD 1c7a557: diagonal pour necks,
fill-keepout rule areas, the router at a quarter to a half less memory, the
probe ladder's rungs and the placement race side by side), 14 boards at a
time under the runner's memory gate: **41 of 109 pass** (v11: 38) in 10375 s
of wall clock for the whole sweep (v11 took about a day at three jobs).
Against v11 (with its reruns spliced): better on 12, same on 77, worse on
12; unconnected 4722 to 4771 in all. The three big losses are known costs
or open bugs: SNSP-CPU-01 116 to 206 (J2's mask opening as its body),
Quanta75 11 to 36 (the knockout plate as an obstacle), the A13 module 26
to 68 (a different race winner; being rerun). Seven error rows: Telemetry
(a degenerate fill-keepout rule area refused by the new lowering), four
panics in `refresh_pour` (SUMEC, coco2, bumwings xiao_sd, Qfwfq; a stale
index into the pour analysis), PolyKybd split72 right at the 1800 s limit
under the sweep's load, the 0xCB panel's kicad-cli refill timeout.

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
CDFER__Business-Cards__Batch_1                                       |              124/204 468 |              125/204 414 |   1709
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |           206/212 23 +5e |                210/212 3 |   1295
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               39/127 253 |               54/127 237 |    939
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |               185/192 26 |               183/192 12 |   1094
oro-os__link__link                                                   |               230/251 70 |               232/251 56 |   1565
anyshake__explorer__Explorer                                         |               140/150 20 |            147/150 7 +1e |   1452
thpoll83__PolyKybd__poly_corne_split42_right                         |            419/420 9 +2e |                418/420 4 |   1704
earth75__atat-1800__ATAT1800                                         |           254/278 40 +5e |          251/278 36 +18e |   1678
CRImier__MyKiCad__protoesp                                           |            93/104 21 +1e |                93/104 18 |   1458
thpoll83__PolyKybd__poly_kybd_split72_left                           |                417/420 6 |                418/420 3 |   1669
greatscottgadgets__hackrf__hackrf-one                                |            318/319 1 +1e |           319/319 0 pass |    359
headblockhead__slab-pcb__interchange-pcb-right                       |              83/83 8 +1e |              83/83 7 +2e |   1303
0xCB-dev__0xCB-1337__1337-v4.0                                       |             47/47 0 pass |             47/47 0 pass |    114
0xCB-dev__0xCB-1337__panel                                           |           188/188 0 pass |           188/188 0 pass |    471
0xCB-dev__0xCB-1337__pcb                                             |             85/85 0 pass |             85/85 0 pass |    141
0xCB-dev__0xCB-Static__0xcb-static                                   |             74/74 0 pass |             74/74 0 pass |     34
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              54/54 0 +1e |              54/54 0 +1e |    237
CDFER__Business-Cards__USB_Keypad                                    |             39/39 0 pass |             39/39 0 pass |    148
CDFER__Business-Cards__WLED_Matrix                                   |                101/104 1 |                101/104 1 |    624
CRImier__MyKiCad__framework_mobo_lefthalf                            |             71/71 0 pass |             71/71 0 pass |    362
CRImier__MyKiCad__zpn_devboard                                       |           167/167 0 pass |           167/167 0 pass |    522
Goga64__ULK__ULK                                                     |              70/70 0 +1e |              70/70 0 +1e |   1052
Goga64__ULK__ULK_sl_PG1316s                                          |              70/70 0 +3e |              70/70 0 +3e |    676
Huaqiu-Electronics__ecad-viewer__video                               |           371/371 0 pass |           371/371 0 pass |   1424
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |            138/138 0 +1e |           138/138 0 pass |     99
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                141/144 5 |                141/144 5 |    982
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |                151/156 8 |                151/156 8 |    676
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |                154/159 8 |                154/159 8 |    614
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |           156/156 0 pass |           156/156 0 pass |    273
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |           154/154 0 pass |           154/154 0 pass |    106
Jana-Marie__ligra__ligra_back                                        |             33/33 0 pass |             33/33 0 pass |     73
Ladniy__jiran-ble-lite__jiran-ble-lite                               |               104/105 20 |               104/105 20 |    862
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |           146/159 17 +3e |           146/159 17 +2e |   1355
Neotron-Compute__Neotron-Pico__neotron-pico                          |           176/190 24 +2e |           176/190 24 +1e |   1329
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |             21/21 0 pass |             21/21 0 pass |    128
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |             19/19 0 pass |             19/19 0 pass |    187
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |               32/184 499 |               32/184 499 |   1122
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  80/82 2 |                  81/82 2 |    852
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                  55/56 4 |                  55/56 4 |    279
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                338/340 2 |                338/340 2 |   1581
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                132/133 1 |                132/133 1 |   1038
bitshiftcrazy__d20_pcb__d20_pcb                                      |             1/19 63 +27e |             1/19 63 +27e |   1425
bitshiftcrazy__spell_tome__spell_tome_bottom                         |             18/18 0 pass |             18/18 0 pass |     69
byrantech__laptop__keyboard                                          |                132/136 3 |                132/136 3 |   1070
byrantech__laptop__motherboard                                       |                234/236 2 |                234/236 2 |   1419
byrantech__laptop__power                                             |           115/115 0 pass |           115/115 0 pass |     80
crmaykish__mackerel-68k__mackerel-08-v1                              |              96/96 0 +5e |              96/96 0 +3e |    440
crmaykish__mackerel-68k__mackerel-10-v1                              |           155/155 0 pass |           155/155 0 pass |   1638
crmaykish__mackerel-68k__mackerel-30-proto                           |           217/217 0 +15e |            217/217 0 +6e |    717
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |           585/585 0 pass |           585/585 0 pass |    145
duckyb__eternal-keypad__eternal-keypad                               |              69/69 0 +2e |             69/69 0 pass |    102
duckyb__urchin__main                                                 |             68/68 0 pass |             68/68 0 pass |     16
ebastler__osprey__osprey_rev_a                                       |                101/103 1 |                101/103 1 |    381
emertcakir__OpenAirScope__OpenAirScope                               |                179/184 5 |                179/184 5 |   1000
hackclub__OnBoard__E-Fidget-Lite                                     |             20/20 0 pass |             20/20 0 pass |     56
hackclub__OnBoard__MotionCubeViewAllForces                           |             20/20 0 pass |             20/20 0 pass |    204
hackclub__OnBoard__PixelWave                                         |            343/343 0 +1e |           343/343 0 pass |    180
hackclub__OnBoard__keyboar_                                          |           102/102 0 pass |           102/102 0 pass |    270
hackclub__OnBoard__krishveercard                                     |                 23/43 21 |                 23/43 21 |    864
hackclub__OnBoard__woagboard                                         |                  90/91 2 |                  90/91 2 |   1357
headblockhead__slab-pcb__slab-pcb-left                               |             76/76 0 pass |             76/76 0 pass |    146
iandchasse__silkscreen-pcb__silkscreen_pcb                           |            113/113 1 +1e |            113/113 1 +1e |    399
little-red-rover__little-red-rover__little_red_rover                 |             64/64 0 pass |             64/64 0 pass |    715
maniekx86__M8SBC-486__homebrew_486                                   |           190/190 0 pass |           190/190 0 pass |   1406
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |            162/162 0 +4e |            162/162 0 +2e |    374
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |            189/190 5 +1e |                189/190 5 |   1059
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |            148/148 0 +6e |            148/148 0 +3e |    781
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |              58/58 0 +2e |             58/58 0 pass |     58
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |             76/76 0 pass |             76/76 0 pass |    550
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |             75/75 0 pass |             75/75 0 pass |    576
tengigabytes__MokyaLora__MokyaLora                                   |           261/262 0 pass |           261/262 0 pass |   1348
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |            8/28 499 +11e |            4/28 499 +12e |   1333
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |            5/28 499 +11e |            4/28 499 +20e |   1584
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |            4/28 499 +10e |             4/28 499 +7e |   1420
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |            5/28 499 +18e |            5/28 499 +34e |   1457
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |            5/28 499 +19e |            3/28 499 +29e |   1540
tomunderwood99__CharlieBoard__Blue_Line                              |             26/26 0 pass |             26/26 0 pass |     31
transistorfet__computie__k30-SBC                                     |           173/173 0 +10e |           173/173 0 +11e |   1206
tubbytwins__bumwings-kbd__bumwings_v001                              |             72/72 0 pass |             72/72 0 pass |    112
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |             70/70 0 pass |             70/70 0 pass |    118
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |             75/75 0 pass |             75/75 0 pass |    284
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |             85/85 0 pass |             85/85 0 pass |     54
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |             81/81 0 pass |             81/81 0 pass |     51
tubbytwins__bumwings-kbd__bumwings_v001_core                         |              98/98 0 +4e |             98/98 0 pass |    345
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |             81/81 0 pass |             81/81 0 pass |    427
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |              78/78 0 +2e |              78/78 0 +1e |    304
tzarc__keyboards__ghoul                                              |                142/148 6 |                142/148 6 |   1062
wntrblm__Castor_and_Pollux__mainboard                                |           126/126 0 +17e |            126/126 0 +2e |    277
zli117__CyberKeeb2040__MainBoard                                     |           118/118 0 pass |           118/118 0 pass |    338
thpoll83__PolyKybd__poly_corne_split42_left                          |                245/247 3 |                246/247 4 |   1617
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |               151/186 82 |               150/186 84 |    896
GlasgowEmbedded__glasgow__glasgow__revC3                             |               177/226 90 |               173/226 93 |   1219
OpenDrone-hw__OpenESC-30x30__4in1                                    |               120/152 84 |               118/152 87 |   1207
hackclub__OnBoard__koeg-board-pcb                                    |           170/175 6 +15e |           168/175 9 +15e |   1472
CRImier__MyKiCad__vaio_re                                            |                262/267 5 |                260/267 9 |   1533
sporkus__capybully_keyboard__capybully                               |                  32/32 1 |                  32/32 5 |   1450
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |               122/152 51 |               128/152 59 |    967
stonedDiscord__MegaDrive__MegaDrive                                  |              274/298 107 |              271/298 119 |   1479
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |          169/172 11 +29e |          147/172 36 +26e |   1480
vd-rd__sbc_allwinner_a13__module                                     |           216/223 26 +1e |           181/223 68 +1e |   1270
stonedDiscord__nonSNES__SNSP-CPU-01                                  |         333/363 116 +58e |              308/363 206 |   1419
0xCB-dev__0xCB-1337__pcb-panel                                       | ERROR (error: kicad-cli timed out aft) | ERROR (error: kicad-cli timed out aft) |   1043
ASH-ART__Qfwfq__qfwfq                                                |             64/64 0 pass | ERROR (note: run with `RUST_BACKTRACE) |    209
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |           108/108 0 pass | ERROR (error: rule area polygon has f) |      0
baldengineer__bit-preserve__coco2                                    |           173/173 0 +12e | ERROR (note: run with `RUST_BACKTRACE) |    273
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |                 57/69 14 | ERROR (note: run with `RUST_BACKTRACE) |    217
ikajdan__katia__katia                                                | ERROR (note: run with `RUST_BACKTRACE) |               148/162 99 |   1783
thpoll83__PolyKybd__poly_kybd_split72_right                          |               419/420 18 | ERROR (timed out after 1800 s) |   1800
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |             89/89 0 pass | ERROR (note: run with `RUST_BACKTRACE) |     68

109 boards in both: better 12, same 77, worse 12; unconnected 4722 -> 4771; passes 38 -> 41

## Freerouting against our cold route on the harvested boards (2026-10-09)

Both route the designer's placement with every track and via removed, 1500 s
each, one board at a time; Freerouting 2.x headless through its CLI
(`benchmarks/github/freerouting.py`), the result imported as a session file
and refilled; counts are KiCad's, errors beyond the designer's board. All 109
boards (`freerouting_compare.py`; our rows are x136 for the first 40 boards
and x155 for the rest, so the two binaries' pour models are mixed on our
side; Freerouting's "failed" rows are a DSN export or session import that
did not work):

board                                                                |                    Freerouting |                          pcb-maker
-------------------------------------------------------------------- | ------------------------------ | ----------------------------------
0xCB-dev__0xCB-1337__1337-v4.0                                       |                7 open +0e 30 s |              47/47 0 open +0e 63 s
0xCB-dev__0xCB-1337__panel                                           |              28 open +0e 185 s |           188/188 0 open +0e 677 s
0xCB-dev__0xCB-1337__pcb                                             |               3 open +86e 33 s |             85/85 0 open +0e 169 s
0xCB-dev__0xCB-1337__pcb-panel                                       |            29 open +201e 603 s |           340/340 0 open +0e 371 s
0xCB-dev__0xCB-Static__0xcb-static                                   |                0 open +0e 10 s |              74/74 0 open +0e 34 s
ASH-ART__Qfwfq__qfwfq                                                |               1 open +0e 100 s |              64/64 0 open +0e 63 s
CDFER__Business-Cards__Batch_1                                       |                 timeout 1500 s |         127/204 499 open +0e 488 s
CDFER__Business-Cards__USB_Cable_Tester__PCB                         |              1 open +172e 24 s |              54/54 0 open +0e 57 s
CDFER__Business-Cards__USB_Keypad                                    |               8 open +39e 35 s |              39/39 0 open +0e 43 s
CDFER__Business-Cards__WLED_Matrix                                   |             12 open +36e 116 s |            101/104 4 open +0e 98 s
CRImier__MyKiCad__framework_mobo_lefthalf                            |              42 open +0e 305 s |              71/71 0 open +0e 64 s
CRImier__MyKiCad__protoesp                                           |              65 open +3e 529 s |           93/104 24 open +0e 232 s
CRImier__MyKiCad__vaio_re                                            |                 timeout 1500 s |         261/267 26 open +0e 1166 s
CRImier__MyKiCad__zpn_devboard                                       |                 no session 3 s |           167/167 0 open +1e 289 s
GlasgowEmbedded__glasgow__glasgow__revC3                             |                 timeout 1500 s |         172/226 86 open +0e 1166 s
Goga64__ULK__ULK                                                     |               0 open +51e 15 s |             70/70 0 open +0e 122 s
Goga64__ULK__ULK_sl_PG1316s                                          |               0 open +10e 30 s |             70/70 0 open +0e 127 s
Huaqiu-Electronics__ecad-viewer__video                               |              1 open +0e 1065 s |          371/371 0 open +0e 1728 s
ISSUIUC__ISS-PCB__BAGEL-MK1                                          |                 timeout 1500 s |           138/138 0 open +0e 122 s
ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado                                |                 timeout 1500 s |           142/144 2 open +0e 608 s
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |             23 open +23e 602 s |           151/156 8 open +0e 482 s
ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA                          |              18 open +0e 923 s |           154/159 8 open +0e 539 s
ISSUIUC__ISS-PCB__MIDAS-MK2                                          |               7 open +0e 199 s |           156/156 0 open +0e 179 s
ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA                      |               5 open +0e 146 s |           154/154 0 open +0e 106 s
Jana-Marie__ligra__ligra_back                                        |                0 open +0e 12 s |              33/33 0 open +0e 69 s
Ladniy__jiran-ble-lite__jiran-ble-lite                               |                5 open +5e 81 s |          104/105 20 open +0e 297 s
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |                 no session 3 s |         148/159 22 open +0e 1000 s
Neotron-Compute__Neotron-Pico__neotron-pico                          |              14 open +2e 144 s |          176/190 24 open +0e 966 s
Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex                         |                0 open +1e 10 s |             21/21 0 open +0e 118 s
Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex |                0 open +0e 35 s |             19/19 0 open +0e 218 s
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |                 timeout 1500 s |           151/152 1 open +0e 482 s
OpenDrone-hw__OpenESC-30x30__4in1                                    |                 timeout 1501 s |           152/152 0 open +0e 442 s
OpenDrone-hw__OpenESC-30x30__4in1-panel                              |                 timeout 1500 s |          49/184 499 open +0e 323 s
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |             8 open +143e 942 s |             80/82 2 open +0e 421 s
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |                 timeout 1500 s |          43/127 266 open +4e 544 s
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |              0 open +18e 194 s |             55/56 4 open +0e 216 s
Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard        |               17 open +0e 95 s |           108/108 0 open +0e 484 s
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |                 timeout 1500 s |          151/186 91 open +0e 709 s
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |                 timeout 1501 s |          338/340 2 open +0e 1298 s
anyshake__explorer__Explorer                                         |              15 open +1e 898 s |           150/150 0 open +0e 214 s
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |                 timeout 1500 s |           132/133 1 open +0e 585 s
baldengineer__bit-preserve__coco2                                    |               0 open +0e 151 s |           173/173 0 open +0e 350 s
bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV                              |            import failed 187 s |             66/69 9 open +0e 599 s
bitshiftcrazy__d20_pcb__d20_pcb                                      |              0 open +151e 10 s |             18/19 3 open +0e 435 s
bitshiftcrazy__spell_tome__spell_tome_bottom                         |                 0 open +2e 6 s |              18/18 0 open +0e 18 s
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |                 timeout 1501 s |         184/192 11 open +2e 1420 s
byrantech__laptop__keyboard                                          |             24 open +11e 644 s |           131/136 4 open +0e 478 s
byrantech__laptop__motherboard                                       |                 timeout 1500 s |           234/236 2 open +0e 997 s
byrantech__laptop__power                                             |              0 open +100e 24 s |            115/115 0 open +0e 91 s
crmaykish__mackerel-68k__mackerel-08-v1                              |               3 open +0e 101 s |             96/96 0 open +0e 230 s
crmaykish__mackerel-68k__mackerel-10-v1                              |               1 open +5e 263 s |           155/155 0 open +0e 544 s
crmaykish__mackerel-68k__mackerel-30-proto                           |             0 open +589e 257 s |          217/217 0 open +13e 501 s
doudar__SmartSpin2k__SmartSpin2k_Panelized                           |                 timeout 1500 s |           585/585 0 open +0e 113 s
duckyb__eternal-keypad__eternal-keypad                               |                0 open +0e 22 s |              69/69 0 open +0e 74 s
duckyb__urchin__main                                                 |                6 open +0e 80 s |              68/68 0 open +0e 13 s
earth75__atat-1800__ATAT1800                                         |                 timeout 1500 s |         255/278 31 open +0e 1800 s
ebastler__osprey__osprey_rev_a                                       |             21 open +6e 1069 s |           101/103 2 open +0e 343 s
emertcakir__OpenAirScope__OpenAirScope                               |               3 open +1e 282 s |           179/184 5 open +0e 727 s
greatscottgadgets__hackrf__hackrf-one                                |             5 open +220e 750 s |           319/319 0 open +0e 441 s
hackclub__OnBoard__E-Fidget-Lite                                     |                 0 open +0e 7 s |              20/20 0 open +0e 15 s
hackclub__OnBoard__MotionCubeViewAllForces                           |               10 open +5e 21 s |              20/20 0 open +0e 77 s
hackclub__OnBoard__PixelWave                                         |                 timeout 1500 s |           343/343 0 open +0e 484 s
hackclub__OnBoard__keyboar_                                          |                 timeout 1500 s |           102/102 0 open +0e 128 s
hackclub__OnBoard__koeg-board-pcb                                    |              export failed 0 s |        169/175 11 open +15e 1220 s
hackclub__OnBoard__krishveercard                                     |               93 open +0e 86 s |            23/43 21 open +0e 699 s
hackclub__OnBoard__woagboard                                         |                1 open +0e 92 s |            90/91 47 open +0e 206 s
headblockhead__slab-pcb__interchange-pcb-right                       |             47 open +30e 413 s |             83/83 8 open +0e 313 s
headblockhead__slab-pcb__slab-pcb-left                               |              54 open +0e 452 s |             76/76 0 open +0e 156 s
iandchasse__silkscreen-pcb__silkscreen_pcb                           |               9 open +26e 93 s |            113/113 0 open +0e 70 s
ikajdan__katia__katia                                                |                 timeout 1500 s |         136/162 84 open +0e 1800 s
little-red-rover__little-red-rover__little_red_rover                 |              8 open +19e 129 s |             64/64 0 open +0e 258 s
maniekx86__M8SBC-486__homebrew_486                                   |               0 open +3e 363 s |           190/190 0 open +0e 724 s
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |              63 open +0e 206 s |           167/172 9 open +0e 675 s
obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized             |                2 open +0e 20 s |           162/162 0 open +0e 102 s
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |             58 open +0e 1210 s |           189/190 7 open +0e 678 s
oro-os__link__link                                                   |                 timeout 1501 s |         248/251 10 open +0e 1734 s
rosco-m68k__rosco_m68k__rosco_m68k__kicad                            |               1 open +6e 184 s |           148/148 0 open +2e 347 s
siderakb__ergo-snm-keyboard__ErgoSNM_keyboard                        |              17 open +1e 215 s |              58/58 0 open +0e 39 s
sporkus__capybully_keyboard__capybully                               |                 timeout 1500 s |            32/32 50 open +0e 622 s
sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys             |               6 open +0e 145 s |              76/76 0 open +0e 91 s
sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre            |               8 open +0e 276 s |              75/75 0 open +0e 78 s
stonedDiscord__MegaDrive__MegaDrive                                  |                 timeout 1500 s |          274/298 95 open +0e 387 s
stonedDiscord__nonSNES__SNSP-CPU-01                                  |                 timeout 1500 s |          361/363 55 open +0e 519 s
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |              2 open +16e 147 s |           212/212 0 open +0e 178 s
tengigabytes__MokyaLora__MokyaLora                                   |                 timeout 1500 s |           261/262 1 open +0e 767 s
thpoll83__PolyKybd__poly_corne_split42_left                          |                 timeout 1500 s |          247/247 0 open +0e 1463 s
thpoll83__PolyKybd__poly_corne_split42_right                         |                 timeout 1500 s |           420/420 0 open +0e 989 s
thpoll83__PolyKybd__poly_kb_molecule_4x4                             |                 timeout 1500 s |            9/28 499 open +0e 870 s
thpoll83__PolyKybd__poly_kb_molecule_4x5                             |                 timeout 1500 s |            6/28 499 open +0e 871 s
thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted                     |                 timeout 1500 s |            6/28 499 open +0e 304 s
thpoll83__PolyKybd__poly_kb_molecule_5x4_wave                        |                 timeout 1500 s |            6/28 499 open +0e 529 s
thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left                   |                 timeout 1500 s |            5/28 499 open +0e 658 s
thpoll83__PolyKybd__poly_kybd_split72_left                           |                 timeout 1500 s |          420/420 0 open +0e 1735 s
thpoll83__PolyKybd__poly_kybd_split72_right                          |                 timeout 1500 s |           420/420 0 open +0e 967 s
tomunderwood99__CharlieBoard__Blue_Line                              |                0 open +0e 10 s |              26/26 0 open +0e 21 s
transistorfet__computie__k30-SBC                                     |               0 open +0e 269 s |           173/173 0 open +0e 528 s
tubbytwins__bumwings-kbd__bumwings_v001                              |              16 open +0e 388 s |              72/72 0 open +0e 75 s
tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd             |              14 open +0e 116 s |              70/70 0 open +0e 55 s
tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd                   |              14 open +0e 270 s |              75/75 0 open +0e 66 s
tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd                   |            import failed 266 s |              85/85 0 open +0e 52 s
tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd             |              17 open +0e 111 s |              81/81 0 open +0e 57 s
tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd                   |              17 open +0e 397 s |              89/89 0 open +0e 86 s
tubbytwins__bumwings-kbd__bumwings_v001_core                         |             20 open +0e 1094 s |             98/98 0 open +0e 102 s
tubbytwins__bumwings-kbd__bumwings_v001_xiao                         |              21 open +0e 762 s |             81/81 0 open +0e 138 s
tubbytwins__bumwings-kbd__bumwings_v001_xiao_s                       |              19 open +0e 401 s |              78/78 0 open +0e 92 s
tzarc__keyboards__ghoul                                              |             10 open +59e 358 s |           142/148 6 open +0e 333 s
vd-rd__sbc_allwinner_a13__module                                     |                 timeout 1500 s |          218/223 22 open +3e 981 s
wntrblm__Castor_and_Pollux__mainboard                                |                1 open +0e 28 s |           126/126 0 open +0e 104 s
zli117__CyberKeeb2040__MainBoard                                     |              13 open +0e 676 s |           118/118 0 open +0e 246 s

109 boards; Freerouting finished 70, timed out 34, failed 5
where Freerouting finished: pcb-maker fewer unconnected on 49, Freerouting fewer on 6; unconnected in all: Freerouting 913, pcb-maker 206; clean boards: Freerouting 8, pcb-maker 50

This supersedes the 2026-09-22 head-to-head below, which used the earlier
adapter.

## Boards harvested from GitHub (added 2026-09-30)

Florian's direction: more benchmarks, from open-source boards on GitHub,
routed on the designer's placement and placed under constraints derived from
it. [benchmarks/github/README.md](../benchmarks/github/README.md) describes
the harvest (`harvest.py search | fetch | boards`): KiCad 7+ boards found by
code search (generator version and package kind) and by repository topic
(files over 300 kB), fetched at the head commit, kept when they are finished
designs of some size. First harvest: 109 boards from 62 repositories, 44
with four copper layers, 12 with six, 2 with eight; 38 with 100 or more
fine pads (a side of 0.3 mm or less). `manifest.json` records origin,
commit, licence and the designer's numbers; the files stay out of the
repository.

Route mode: `benchmarks/corpus/run.py <out> --corpus benchmarks/github/boards.json --skip-layout`,
summarised by cause with `benchmarks/github/summary.py <out>`. Layout tasks:
`benchmarks/agent-tasks/from_pcbench.py <tasks> --corpus benchmarks/github/boards.json`
then `benchmarks/agent-tasks/run.py <out> --tasks <tasks>/tasks.json`.
To measure a change (short tier, route mode from fixed placements, spliced
reruns, the objectives line), see benchmarks/agent-tasks/README.md, "How to
measure".

From the schematic (2026-10-15): `harvest_schematics.py` fetched the
schematics and project libraries of 79 of the 109 boards; the schematic
tier (`run.py --tier schematic`, benchmarks/agent-tasks/README.md, "The
schematic tier") imports 12 of them with `import-kicad-netlist` and lays
them out at a third of the budget: 4/12 pass on x1002, 5/12 with the
branch's import fixes (x1030); where the import and the libraries agree
with the designer's board they do as the layout tier does.

What the first boards found, before any full run finished (fixed the same
day): chamfered roundrect pads were refused (4 boards), a rule area's holes
were read as more keepouts (Glasgow's rim keepout covered the whole board,
0/226), and the ladder's first rung could spend the whole budget on a
hopeless configuration (video).

### Second sweep, 92 boards, with the day's fixes (`pcb-maker-x8`, 900 s each, 2026-10-01)

17 clean, 42 open, 11 with copper errors beyond the designer's (KiCad DRC),
1 mismatch (KiCad finds an unconnected item the router does not), 21
errors. Corrected on 2026-10-03: the first version of this table said 25
clean, because `summary.py` read a key the runner does not write and so
never counted DRC errors (one board it called clean had 128 hole
clearance errors, the copper-to-drill rule the router did not know yet).
Via counts are mostly below the designer's. 17 of the 109 boards were
skipped (strip failures since fixed).

| Board | Layers | Footprints | Result | Vias (designer) | Time |
| --- | --- | --- | --- | --- | --- |
| 0xCB-dev__0xCB-Static__0xcb-static | 2 | 129 | clean: 74/74, 2 vias (designer 0), 25 s | 0 | |
| CRImier__MyKiCad__framework_mobo_lefthalf | 6 | 45 | clean: 71/71, 76 vias (designer 203), 101 s | 203 | |
| ISSUIUC__ISS-PCB__BAGEL-MK1 | 4 | 199 | clean: 138/138, 151 vias (designer 525), 395 s | 525 | |
| Jana-Marie__ligra__ligra_back | 2 | 61 | clean: 33/33, 11 vias (designer 251), 62 s | 251 | |
| Open-Muscle__OpenMuscle-FlexGrid__OM-60-Flex | 2 | 62 | clean: 21/21, 39 vias (designer 120), 146 s | 120 | |
| Open-Muscle__OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex | 2 | 61 | clean: 19/19, 65 vias (designer 120), 275 s | 120 | |
| bitshiftcrazy__d20_pcb__d20_pcb | 2 | 39 | clean: 19/19, 34 vias (designer 46), 57 s | 46 | |
| crmaykish__mackerel-68k__mackerel-08-v1 | 4 | 86 | clean: 96/96, 64 vias (designer 208), 488 s | 208 | |
| duckyb__urchin__main | 2 | 122 | clean: 68/68, 7 vias (designer 53), 14 s | 53 | |
| hackclub__OnBoard__E-Fidget-Lite | 2 | 29 | clean: 20/20, 7 vias (designer 0), 7 s | 0 | |
| hackclub__OnBoard__MotionCubeViewAllForces | 2 | 30 | clean: 20/20, 35 vias (designer 29), 56 s | 29 | |
| hackclub__OnBoard__keyboar_ | 2 | 158 | clean: 104/104, 20 vias (designer 0), 176 s | 0 | |
| sporkus__le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 2 | 118 | clean: 76/76, 61 vias (designer 0), 175 s | 0 | |
| sporkus__le_chiffre_keyboard_stm32__stm32_hotswap_chiffre | 2 | 110 | clean: 75/75, 42 vias (designer 0), 111 s | 0 | |
| tomunderwood99__CharlieBoard__Blue_Line | 2 | 25 | clean: 26/26, 26 vias (designer 28), 4 s | 28 | |
| tubbytwins__bumwings-kbd__bumwings_v001 | 2 | 120 | clean: 72/72, 22 vias (designer 0), 93 s | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001_core | 2 | 153 | clean: 98/98, 71 vias (designer 0), 119 s | 0 | |
| CRImier__MyKiCad__vaio_re | 4 | 289 | open: 37 open; 230/267, 397 vias (designer 523), 464 s | 523 | |
| CRImier__MyKiCad__zpn_devboard | 4 | 168 | open: 1 open; 166/167, 149 vias (designer 298), 641 s | 298 | |
| GlasgowEmbedded__glasgow__glasgow__revC3 | 4 | 272 | open: 61 open; 165/226, 206 vias (designer 0), 454 s | 0 | |
| Huaqiu-Electronics__ecad-viewer__video | 4 | 189 | open: 48 open; 323/371, 990 vias (designer 808), 846 s | 808 | |
| ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado | 4 | 212 | open: 9 open; 135/144, 266 vias (designer 820), 322 s | 820 | |
| ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 4 | 216 | open: 5 open; 151/156, 131 vias (designer 0), 73 s | 0 | |
| ISSUIUC__ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA | 4 | 221 | open: 5 open; 154/159, 139 vias (designer 0), 45 s | 0 | |
| ISSUIUC__ISS-PCB__MIDAS-MK2 | 4 | 217 | open: 1 open; 155/156, 167 vias (designer 401), 157 s | 401 | |
| ISSUIUC__ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA | 4 | 219 | open: 1 open; 153/154, 163 vias (designer 432), 337 s | 432 | |
| Ladniy__jiran-ble-lite__jiran-ble-lite | 2 | 154 | open: 1 open; 104/105, 54 vias (designer 0), 48 s | 0 | |
| MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub | 4 | 235 | open: 56 open; 103/159, 102 vias (designer 331), 224 s | 331 | |
| Neotron-Compute__Neotron-Pico__neotron-pico | 4 | 242 | open: 14 open; 176/190, 155 vias (designer 0), 505 s | 0 | |
| OpenDrone-hw__OpenFC-Lite__OpenFC | 6 | 164 | open: 1 open; 81/82, 121 vias (designer 596), 140 s | 596 | |
| OpenDrone-hw__OpenRX__OpenRX-panel-rev2 | 6 | 254 | open: 50 open; 53/103, 125 vias (designer 538), 465 s | 538 | |
| Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1 | 4 | 92 | open: 2 open; 54/56, 44 vias (designer 334), 17 s | 334 | |
| Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard | 4 | 143 | open: 3 open; 105/108, 71 vias (designer 269), 148 s | 269 | |
| Twisted-Fields__rp2040-motor-controller__RP2040_base | 4 | 325 | open: 71 open; 115/186, 233 vias (designer 553), 82 s | 553 | |
| apfaudio__eurorack-pmod__eurorack-pmod-pcb | 6 | 155 | open: 2 open; 131/133, 133 vias (designer 331), 242 s | 331 | |
| baldengineer__bit-preserve__coco2 | 2 | 156 | open: 1 open; 172/173, 315 vias (designer 0), 713 s | 0 | |
| bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV | 4 | 122 | open: 12 open; 57/69, 89 vias (designer 541), 33 s | 541 | |
| briskspirit__Sisu_SSE-9__Sisu_SSE-9 | 6 | 296 | open: 8 open; 184/192, 239 vias (designer 1076), 492 s | 1076 | |
| byrantech__laptop__keyboard | 2 | 220 | open: 4 open; 132/136, 173 vias (designer 571), 276 s | 571 | |
| byrantech__laptop__motherboard | 6 | 264 | open: 3 open; 233/236, 378 vias (designer 734), 641 s | 734 | |
| ebastler__osprey__osprey_rev_a | 2 | 160 | open: 3 open; 100/103, 120 vias (designer 0), 87 s | 0 | |
| emertcakir__OpenAirScope__OpenAirScope | 4 | 282 | open: 6 open; 178/184, 146 vias (designer 473), 254 s | 473 | |
| greatscottgadgets__hackrf__hackrf-one | 4 | 437 | open: 3 open; 316/319, 396 vias (designer 0), 664 s | 0 | |
| hackclub__OnBoard__krishveercard | 4 | 39 | open: 14 open; 29/43, 38 vias (designer 87), 167 s | 87 | |
| headblockhead__slab-pcb__interchange-pcb-right | 2 | 178 | open: 2 open; 81/83, 242 vias (designer 203), 236 s | 203 | |
| headblockhead__slab-pcb__slab-pcb-left | 2 | 128 | open: 2 open; 76/78, 192 vias (designer 239), 192 s | 239 | |
| little-red-rover__little-red-rover__little_red_rover | 2 | 65 | open: 1 open; 63/64, 110 vias (designer 0), 76 s | 0 | |
| maniekx86__M8SBC-486__homebrew_486 | 4 | 130 | open: 2 open; 188/190, 1102 vias (designer 1150), 717 s | 1150 | |
| obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized | 2 | 276 | open: 25 open; 147/172, 53 vias (designer 95), 769 s | 95 | |
| obsilab__Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized | 2 | 253 | open: 1 open; 161/162, 31 vias (designer 0), 290 s | 0 | |
| ohdsp__DSP-ADAU1452__DSP-ADAU1452 | 4 | 377 | open: 27 open; 163/190, 41 vias (designer 355), 94 s | 355 | |
| rosco-m68k__rosco_m68k__rosco_m68k__kicad | 4 | 140 | open: 1 open; 147/148, 416 vias (designer 402), 648 s | 402 | |
| stonedDiscord__nonSNES__SNSP-CPU-01 | 2 | 400 | open: 2 open; 361/363, 1202 vias (designer 1197), 319 s | 1197 | |
| stonedDiscord__nonSNES__SNSP-CPU-1CHIP | 2 | 241 | open: 1 open; 211/212, 359 vias (designer 686), 555 s | 686 | |
| tengigabytes__MokyaLora__MokyaLora | 6 | 443 | open: 4 open; 258/262, 172 vias (designer 836), 611 s | 836 | |
| thpoll83__PolyKybd__poly_kb_molecule_5x2_shifted | 2 | 392 | open: 23 open; 4/27, 199 vias (designer 0), 701 s | 0 | |
| tzarc__keyboards__ghoul | 2 | 191 | open: 6 open; 142/148, 244 vias (designer 0), 57 s | 0 | |
| vd-rd__sbc_allwinner_a13__module | 4 | 179 | open: 41 open; 182/223, 361 vias (designer 501), 366 s | 501 | |
| wntrblm__Castor_and_Pollux__mainboard | 4 | 235 | open: 1 open; 125/126, 121 vias (designer 358), 234 s | 358 | |
| 0xCB-dev__0xCB-1337__1337-v4.0 | 2 | 64 | drc: {'starved_thermal': 3, 'copper_sliver': 6, 'track_dangling': 1}; 47/47, 44 vias (designer 0), 161 s | 0 | |
| 0xCB-dev__0xCB-1337__pcb | 4 | 119 | drc: {'connection_width': 3}; 85/85, 87 vias (designer 0), 107 s | 0 | |
| anyshake__explorer__Explorer | 2 | 266 | drc: {'starved_thermal': 1}; 150/150, 117 vias (designer 1057), 265 s | 1057 | |
| bitshiftcrazy__spell_tome__spell_tome_bottom | 2 | 25 | drc: {'starved_thermal': 2}; 18/18, 7 vias (designer 6), 15 s | 6 | |
| byrantech__laptop__power | 4 | 179 | drc: {'hole_clearance': 93}; 115/115, 91 vias (designer 167), 285 s | 167 | |
| hackclub__OnBoard__PixelWave | 2 | 663 | drc: {'starved_thermal': 5, 'track_dangling': 8}; 343/343, 708 vias (designer 705), 117 s | 705 | |
| iandchasse__silkscreen-pcb__silkscreen_pcb | 2 | 188 | drc: {'starved_thermal': 2}; 113/113, 138 vias (designer 192), 28 s | 192 | |
| siderakb__ergo-snm-keyboard__ErgoSNM_keyboard | 2 | 98 | drc: {'clearance': 8}; 58/58, 12 vias (designer 0), 39 s | 0 | |
| sporkus__capybully_keyboard__capybully | 4 | 114 | drc: {'copper_sliver': 3, 'starved_thermal': 2, 'via_dangling': 4}; 32/32, 19 vias (designer 0), 236 s | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001_xiao | 2 | 127 | drc: {'clearance': 15}; 81/81, 47 vias (designer 0), 127 s | 0 | |
| zli117__CyberKeeb2040__MainBoard | 2 | 182 | drc: {'hole_to_hole': 13, 'via_dangling': 1}; 118/118, 169 vias (designer 0), 474 s | 0 | |
| duckyb__eternal-keypad__eternal-keypad | 2 | 93 | mismatch: 2 unconnected in KiCad; 63/63, 35 vias (designer 0), 73 s | 0 | |
| antmicro__jetson-nano-baseboard__jetson-nano-baseboard | 8 | 391 | error: iteration 24: rerouted 31, conflicted 28, present 8417.06, 613.61s (search 609.15s, stamp 4.41s, 887 | 794 | |
| crmaykish__mackerel-68k__mackerel-10-v1 | 4 | 114 | error: iteration 18: rerouted 2, conflicted 0, present 738.95, 284.20s (search 618.13s, stamp 23.70s, 12682 | 200 | |
| crmaykish__mackerel-68k__mackerel-30-proto | 4 | 133 | error: iteration 25: rerouted 2, conflicted 0, present 10000.00, 235.74s (search 544.77s, stamp 14.97s, 156 | 294 | |
| earth75__atat-1800__ATAT1800 | 4 | 342 | error: iteration 33: rerouted 58, conflicted 57, present 10000.00, 613.74s (search 598.48s, stamp 15.17s, 5 | 680 | |
| hackclub__OnBoard__koeg-board-pcb | 4 | 290 | error: error: kicad-cli failed with exit status: 3: Failed to load board: One or more items were found on u | 417 | |
| oro-os__link__link | 8 | 381 | error: cleanup: 329 improvements, 181.95s | 806 | |
| stonedDiscord__MegaDrive__MegaDrive | 2 | 291 | error: pour GND: 2242 pieces, 70 stranded terminals (62 touch a piece) | 785 | |
| thpoll83__PolyKybd__poly_corne_split42_right | 4 | 573 | error: iteration 79: rerouted 8, conflicted 7, present 10000.00, 602.03s (search 586.43s, stamp 15.18s, 131 | 1372 | |
| thpoll83__PolyKybd__poly_kb_molecule_4x4 | 2 | 576 | error: iteration 26: rerouted 26, conflicted 26, present 10000.00, 315.43s (search 307.18s, stamp 8.12s, 23 | 0 | |
| thpoll83__PolyKybd__poly_kb_molecule_4x5 | 2 | 716 | error: iteration 26: rerouted 26, conflicted 26, present 10000.00, 453.07s (search 442.42s, stamp 10.47s, 2 | 0 | |
| thpoll83__PolyKybd__poly_kb_molecule_5x4_wave | 2 | 393 | error: iteration 26: rerouted 26, conflicted 26, present 10000.00, 349.15s (search 339.69s, stamp 9.33s, 22 | 0 | |
| thpoll83__PolyKybd__poly_kb_molecule_7x5_wave_left | 2 | 629 | error: iteration 24: rerouted 26, conflicted 26, present 8417.06, 626.26s (search 612.42s, stamp 13.62s, 30 | 0 | |
| thpoll83__PolyKybd__poly_kybd_split72_left | 4 | 573 | error: iteration 49: rerouted 2, conflicted 0, present 10000.00, 351.86s (search 343.13s, stamp 8.51s, 1082 | 1354 | |
| thpoll83__PolyKybd__poly_kybd_split72_right | 4 | 573 | error: iteration 79: rerouted 4, conflicted 4, present 10000.00, 488.28s (search 477.32s, stamp 10.63s, 113 | 1368 | |
| transistorfet__computie__k30-SBC | 4 | 88 | error: iteration 18: rerouted 4, conflicted 0, present 738.95, 480.52s (search 665.08s, stamp 10.06s, 12090 | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R55_rp2040zero_sd | 2 | 120 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R55_xiao_sd | 2 | 121 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R64_nano_sd | 2 | 135 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 2 | 130 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001R64_xiao_sd | 2 | 132 | error: error: unknown copper layer "In1.Cu" | 0 | |
| tubbytwins__bumwings-kbd__bumwings_v001_xiao_s | 2 | 122 | error: error: unknown copper layer "In1.Cu" | 0 | |

92 boards: clean 17, open 42, drc 11, mismatch 1, error 21


### First sweep, 38 boards, before the day's fixes (`pcb-maker-x4`, 900 s each)

Stopped after 38 boards once its failure classes were clear, so the machine
could rerun everything with the fixes (`build/github-route-v2`, table to
follow). Times are under a loaded machine.

| Board | Layers | Footprints | Result | Vias (designer) | Time |
| --- | --- | --- | --- | --- | --- |
| ISSUIUC__ISS-PCB__BAGEL-MK1 | 4 | 199 | clean: 138/138, 156 vias (designer 525), 420 s | 525 | |
| CRImier__MyKiCad__framework_mobo_lefthalf | 6 | 45 | open: 1 open; 70/71, 81 vias (designer 203), 41 s | 203 | |
| CRImier__MyKiCad__vaio_re | 4 | 289 | open: 31 open; 236/267, 405 vias (designer 523), 485 s | 523 | |
| CRImier__MyKiCad__zpn_devboard | 4 | 168 | open: 1 open; 166/167, 164 vias (designer 298), 348 s | 298 | |
| GlasgowEmbedded__glasgow__glasgow | ? | ? | open: 226 open; 0/226, 0 vias (designer None), 2 s | ? | |
| GlasgowEmbedded__glasgow__glasgow | ? | ? | open: 226 open; 0/226, 0 vias (designer None), 2 s | ? | |
| ISSUIUC__ISS-PCB__BAGEL-MK1.1-Avocado | 4 | 212 | open: 10 open; 134/144, 269 vias (designer 820), 170 s | 820 | |
| Neotron-Compute__Neotron-Pico__neotron-pico | 4 | 242 | open: 14 open; 176/190, 185 vias (designer 0), 298 s | 0 | |
| OpenDrone-hw__OpenFC-Lite__OpenFC | 6 | 164 | open: 1 open; 81/82, 121 vias (designer 596), 82 s | 596 | |
| OpenDrone-hw__OpenRX__OpenRX-panel-rev2 | 6 | 254 | open: 47 open; 56/103, 117 vias (designer 538), 482 s | 538 | |
| Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1 | 4 | 92 | open: 2 open; 54/56, 44 vias (designer 334), 18 s | 334 | |
| Spaceflight-Rocketry-Giessen-e-V__Telemetry__TelemetryOnboard | 4 | 143 | open: 3 open; 105/108, 73 vias (designer 269), 82 s | 269 | |
| Twisted-Fields__rp2040-motor-controller__RP2040_base | 4 | 325 | open: 71 open; 115/186, 243 vias (designer 553), 98 s | 553 | |
| apfaudio__eurorack-pmod__eurorack-pmod-pcb | 6 | 155 | open: 2 open; 131/133, 130 vias (designer 331), 279 s | 331 | |
| bismarx-v1__Sumec-MiniSumo__SUMEC_MK_IV | 4 | 122 | open: 12 open; 57/69, 83 vias (designer 541), 64 s | 541 | |
| briskspirit__Sisu_SSE-9__Sisu_SSE-9 | 6 | 296 | open: 10 open; 182/192, 466 vias (designer 1076), 303 s | 1076 | |
| byrantech__laptop__motherboard | 6 | 264 | open: 3 open; 233/236, 378 vias (designer 734), 656 s | 734 | |
| emertcakir__OpenAirScope__OpenAirScope | 4 | 282 | open: 6 open; 178/184, 150 vias (designer 473), 328 s | 473 | |
| greatscottgadgets__hackrf__hackrf-one | 4 | 437 | open: 4 open; 315/319, 394 vias (designer 0), 150 s | 0 | |
| hackclub__OnBoard__krishveercard | 4 | 39 | open: 30 open; 13/43, 27 vias (designer 87), 2 s | 87 | |
| maniekx86__M8SBC-486__homebrew_486 | 4 | 130 | open: 6 open; 184/190, 1018 vias (designer 1150), 713 s | 1150 | |
| ohdsp__DSP-ADAU1452__DSP-ADAU1452 | 4 | 377 | open: 28 open; 162/190, 48 vias (designer 355), 166 s | 355 | |
| wntrblm__Castor_and_Pollux__mainboard | 4 | 235 | open: 1 open; 125/126, 125 vias (designer 358), 239 s | 358 | |
| 0xCB-dev__0xCB-1337__pcb | 4 | 119 | error: error: chamfered roundrect pads have no exact geometry lowering | 0 | |
| Huaqiu-Electronics__ecad-viewer__video | 4 | 189 | error: cleanup: 582 improvements, 72.89s | 808 | |
| ISSUIUC__ISS-PCB__MIDAS-MK2 | 4 | 217 | error: error: chamfered roundrect pads have no exact geometry lowering | 401 | |
| MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub | 4 | 235 | error: error: chamfered roundrect pads have no exact geometry lowering | 331 | |
| antmicro__jetson-nano-baseboard__jetson-nano-baseboard | 8 | 391 | error: iteration 17: rerouted 72, conflicted 60, present 492.63, 607.19s (search 603.21s, stamp 3.93s, 8240 | 794 | |
| byrantech__laptop__power | 4 | 179 | error: error: chamfered roundrect pads have no exact geometry lowering | 167 | |
| crmaykish__mackerel-68k__mackerel-30-proto | 4 | 133 | error: iteration 20: rerouted 6, conflicted 0, present 1662.63, 230.77s (search 509.21s, stamp 14.17s, 1548 | 294 | |
| earth75__atat-1800__ATAT1800 | 4 | 342 | error: iteration 32: rerouted 56, conflicted 53, present 10000.00, 605.68s (search 590.51s, stamp 15.06s, 4 | 680 | |
| hackclub__OnBoard__koeg-board-pcb | 4 | 290 | error: error: chamfered roundrect pads have no exact geometry lowering | 417 | |
| oro-os__link__link | 8 | 381 | error: error: chamfered roundrect pads have no exact geometry lowering | 806 | |
| tengigabytes__MokyaLora__MokyaLora | 6 | 443 | error: error: per-layer roundrect padstacks have no exact geometry lowering | 836 | |
| thpoll83__PolyKybd__poly_corne_split42_right | 4 | 573 | error: error: chamfered roundrect pads have no exact geometry lowering | 1372 | |
| thpoll83__PolyKybd__poly_kybd_split72_left | 4 | 573 | error: error: chamfered roundrect pads have no exact geometry lowering | 1354 | |
| thpoll83__PolyKybd__poly_kybd_split72_right | 4 | 573 | error: error: chamfered roundrect pads have no exact geometry lowering | 1368 | |
| vd-rd__sbc_allwinner_a13__module | 4 | 179 | error: error: chamfered roundrect pads have no exact geometry lowering | 501 | |

38 boards: clean 1, open 22, drc 0, mismatch 0, error 15


## Breadboard fence boards (added 2026-09-25)

[`benchmarks/fence/`](../benchmarks/fence/README.md) holds the boards of a
separate hardware project that uses pcb-maker as its placer and router: a
dense 76 × 31 mm board where 30 row nets each cross between a resistor
network and one of two LQFP-44 crosspoint switches. Five of them are
`corpus.json` entries (`fence-*`): the current 4-layer boards (complete so
far), the same boards on 2 layers (not routable so far) and a footprint
keepout test. Its README also carries an issue log of what pcb-maker got
wrong on these boards with a benchmark and a pass criterion for each, and a
re-run of everything on 2026-10-07: footprint keepouts, exclusive planes,
constraints and pin swapping all work there now; the 31 mm boards still do
not route on 2 layers (8 mm taller they do, with the GND pour in pieces),
silkscreen stays unplaced.

**v5 (2026-10-21):** `benchmarks/fence/v5-4layer/fence` (the shipped board:
41 LEDs in a 2.54 mm row, a stack header, three `keepout` constraints;
complete with `router-seeds.json`, 134/134, 196 vias) and
`v5-2layer/fence` (the same board on 2 layers at 34 mm with 0.127 mm rules:
7 open at best, the B.Cu GND pour in 43 pieces; 12 mm taller it completes
with the pour in 37 pieces). Corpus entries `fence-v5-4l` and `fence-v5-2l`.
The pass criterion for the 2-layer board includes the pour
(`benchmarks/fence/pour-pieces.py`). New issues 13–17 in that README:
vias inside LED pads, moved silkscreen text, keepouts judging fixed parts,
`repair-kicad-silkscreen` on a `gr_rect` outline, and no way to keep a
2-layer pour whole.

## Layout sweep v15 (2026-10-14): the spoke model, solid bodies, moving rule areas

`pcb-maker-x723` (b63a604: the KiCad-exact thermal spoke model with manual
spokes, island vias and stubs, the mask-opening clearance from the project,
courtyard-less pin-row footprints blocking with their pad box, mask-only
pads as openings, footprint rule areas moving with their footprint,
`keep_on_pieces`, far-side and inner-layer pad keepouts) on the v6 tasks, 6
jobs, 17910 s of wall clock: **45 of 109 pass** (v14: 34); against v14
better on 20, same on 63, worse on 16; 3700 to 3829 open in all (video's
499 is KiCad's cap; the 0xCB panel 129 to 210, the allwinner module 8 to
81 and RP2040_base 100 to 173 carry the rise). Eleven boards that failed
only on starved thermals or a stray error pass now (PixelWave, k30,
mackerel-08 and -30, rosco, Castor, Telemetry, MokyaLora, spell_tome, two
bumwings), zpn_devboard routes clean (its parts had spread over an empty
breakaway), and link finishes inside the limit. Error rows: the five
molecule panels and the OpenESC 4in1 panel find no legal placement; katia,
MegaDrive and the 0xCB pcb-panel hit the 1800 s limit. The worse rows are
the crowded boards (module, jetson, vaio, woagboard, protoesp, the 0xCB
panel, video), where a placement change flips the result by tens of
connections run to run (the routing-room agent's seed pairs; video is not
even deterministic for one placement, under investigation), and where the
pin-row rule now forbids the parts-inside-bodies placements that v14
counted.

Better (first 20 rows of `compare.py`):

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
OpenDrone-hw__OpenESC-30x30__4in1                                    |               61/152 239 |               97/152 124 |    935
CRImier__MyKiCad__zpn_devboard                                       |               128/167 71 |           167/167 0 pass |    855
ohdsp__DSP-ADAU1452__DSP-ADAU1452                                    |         118/190 167 +49e |         146/190 105 +59e |   1713
OpenDrone-hw__OpenESC-20x20__4in1-mini                               |                95/152 96 |               123/152 38 |    675
OpenDrone-hw__OpenRX__OpenRX-panel-rev2                              |               44/127 257 |               56/127 236 |   1263
thpoll83__PolyKybd__poly_corne_split42_right                         |           393/420 57 +1e |               398/420 46 |   1719
briskspirit__Sisu_SSE-9__Sisu_SSE-9                                  |           188/192 18 +1e |                188/192 9 |   1206
apfaudio__eurorack-pmod__eurorack-pmod-pcb                           |               116/133 27 |               125/133 20 |   1014
Seeed-Studio__OSHW-reCamera-Series__reCamera_S101_v1.1               |                 53/56 10 |                  55/56 4 |    402
CDFER__Business-Cards__WLED_Matrix                                   |             99/104 9 +8e |           102/104 4 +13e |    690
obsilab__Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized              |         146/172 128 +70e |         148/172 123 +73e |   1708
thpoll83__PolyKybd__poly_kybd_split72_left                           |               394/420 55 |               396/420 52 |   1696
ISSUIUC__ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA                      |               150/156 12 |               151/156 11 |    743
OpenDrone-hw__OpenFC-Lite__OpenFC                                    |                  81/82 2 |                  81/82 1 |    300
byrantech__laptop__motherboard                                       |                233/236 3 |                234/236 2 |   1602
byrantech__laptop__power                                             |                112/115 3 |                113/115 2 |   1320
emertcakir__OpenAirScope__OpenAirScope                               |                178/184 6 |                179/184 5 |   1190
greatscottgadgets__hackrf__hackrf-one                                |            318/319 1 +6e |            319/319 0 +1e |   1044
sporkus__capybully_keyboard__capybully                               |                  32/32 2 |                  32/32 1 |   1777
tzarc__keyboards__ghoul                                              |                142/148 7 |            142/148 6 +1e |   1762

Worse:

board                                                                |                   before |                    after |      s
-------------------------------------------------------------------- | ------------------------ | ------------------------ | ------
hackclub__OnBoard__koeg-board-pcb                                    |          166/175 11 +15e |          165/175 12 +17e |   1711
Goga64__ULK__ULK                                                     |                  68/70 2 |              66/70 4 +1e |   1688
ebastler__osprey__osprey_rev_a                                       |            102/103 1 +3e |            100/103 3 +2e |    822
MbFredys__PCB-Modular-Multi-Protocol-Hub__Hub                        |               139/159 32 |               137/159 35 |    910
stonedDiscord__nonSNES__SNSP-CPU-1CHIP                               |           210/212 2 +16e |           206/212 5 +20e |   1069
earth75__atat-1800__ATAT1800                                         |           242/278 49 +7e |           244/278 53 +7e |   1484
stonedDiscord__nonSNES__SNSP-CPU-01                                  |         302/363 239 +22e |         300/363 246 +16e |   1702
GlasgowEmbedded__glasgow__glasgow__revC3                             |              113/226 254 |              112/226 263 |   1318
hackclub__OnBoard__woagboard                                         |            85/91 11 +34e |            76/91 26 +26e |   1046
antmicro__jetson-nano-baseboard__jetson-nano-baseboard               |           333/340 9 +11e |           325/340 25 +1e |   1662
CRImier__MyKiCad__protoesp                                           |            82/104 66 +5e |            76/104 83 +2e |   1342
CRImier__MyKiCad__vaio_re                                            |           240/267 81 +1e |              225/267 101 |   1315
Twisted-Fields__rp2040-motor-controller__RP2040_base                 |              148/186 100 |              151/186 173 |   1015
vd-rd__sbc_allwinner_a13__module                                     |            221/223 8 +1e |           186/223 81 +1e |   1227
0xCB-dev__0xCB-1337__panel                                           |          145/188 129 +2e |              137/188 210 |   1720
Huaqiu-Electronics__ecad-viewer__video                               |              253/371 315 |              252/371 499 |   1673

## Freerouting against x1367 (2026-10-21)

**Complete: all 109 boards** (measured 2026-10-21 to 2026-10-23), and the
ten-board speed study. Our side is pcb-maker-x1367 throughout, for
comparability with the rows Freerouting was given its wall from; main has
moved on since (x1573 and later: the memory ledger, pour integrity off by
default, the silkscreen placement, router memory, trace aesthetics and the
trouble repair default; see [changelog.md](changelog.md)), so these are not
main's numbers.

Same placements on both sides: route mode on the v17 layout placements
(`build/github-layout-v17/<task>/layout/placed`: the layout's parts, the
designer's keepouts and pours, no tracks), tasks v7, the task budgets.
Ours: `run.py --route-from build/github-layout-v17` with
pcb-maker-x1367, 8 jobs x 2 threads at a load of 12 to 50
(`build/fr2-route-x1367.log`). Rows the run lost were rerun alone and
spliced (`build/fr2-route-final.log`): bumwings_v001_core and ghoul
(killed by the kernel's OOM killer, exit -9: such a row still carries a
result board from the ladder's last best attempt and is not the binary's
answer) and the 0xCB pcb-panel on x1367; the rows cut by the wall clock or
the harness timeout (katia, Batch_1, the 0xCB panel, poly_kb_molecule_4x4)
on x1371, which stretches the wall guard under load and differs from x1367
only in work accounting. katia hits the harness's 2700 s timeout again on
x1371 alone (8.8 GB peak) and the pcb-panel's KiCad refill times out at
900 s again: both stay rows without a result. No row with exit -9 or 137
is left.

Freerouting 2.2.4 headless (`benchmarks/github/freerouting_route_mode.py`,
freerouting.py's pipeline: KiCad's DSN export, the class rules raised to the
board minimums, its batch passes, KiCad's session import), `-mt 1`,
`-Xmx4g`, with its own job timeout (`--router.job_timeout`) set to the wall
our row of the board took: its passes until it stops (it stops by itself
when a pass no longer improves) or that wall, and then it writes the session
of what it has; the process is killed at that wall plus 300 s. The
designer's pours stay on the board: KiCad's DSN export makes them planes,
Freerouting reaches them with vias, and KiCad's DRC refills them, as on our
side (where the router fills them itself). Both results are judged by
run.py's own functions: KiCad's DRC (`drc-kicad-board`, zones refilled),
the pass rule (no unconnected item, no copper error beyond the designer's
count of its type) and the strict rule (only the findings the designer's
stripped board has at the same place are subtracted). Wall and CPU are each
side's whole pipeline: ours the route process with its KiCad checks,
Freerouting's the JVM plus the DSN export, the session import and the DRC.
Freerouting ran three boards at a time beside other agents' runs, at a
load of 1 to 49 (`build/fr2-freerouting-x1367.log`).

**Summary** (`benchmarks/github/freerouting_route_compare.py
build/fr2-route-final.log build/fr2-freerouting-x1367`):

| | pcb-maker x1367 | Freerouting 2.2.4 |
| --- | ---: | ---: |
| boards with a KiCad-checked result (of 109) | 107 | 75 |
| pass (designer-subtracted per type) | 51 | 3 |
| strict pass | 44 | 3 |
| better on KiCad's unconnected, then errors, then strict findings | 101 | 4 (4 ties) |
| 74 boards both have: KiCad unconnected | 2 700 | 9 619 |
| same: counting errors (pass rule) | 141 | 3 016 |
| same: strict new findings | 1 254 | 4 603 |
| same: vias | 18 799 | 7 822 |
| same: track length | 538.0 m | 366.4 m |
| same: CPU seconds | 82 286 | 96 899 |
| per board, Freerouting over ours: CPU median / wall median | | x1.31 / x1.01 |
| stopped by the wall (its job timeout) | | 70 |

On the 74 boards both finish Freerouting leaves 3.6 times our unconnected
items and 21 times our counting errors; its copper is shorter and has
fewer vias because so much stays open. The three boards it passes
(0xcb-static, OM-60-Flex, CharlieBoard Blue_Line) are three of the four
ties: ours strict-passes them too, on 0xcb-static and Blue_Line in less
wall and a quarter or less of its CPU, on OM-60-Flex in 121 s against its 55 s
(148 against 129 CPU seconds). Freerouting at `-mt 1` still uses a median
2.2 CPU seconds per wall second (0.8 to 4.4: the JVM's compiler and
collector threads); ours ran at two search threads.

Freerouting's 34 boards without a result:
- 25 it did not stop within its job timeout plus 300 s (a pass does not
  look at the timeout; on a loaded machine that includes small boards):
  laptop motherboard, OpenESC 4in1-panel, 4in1 and 4in1-mini, Sisu_SSE-9,
  eurorack-pmod, vaio_re, poly_corne_split42_left, BAGEL-MK1 and
  MK1.1-Avocado, MIDAS-MK2 and MK1.1, koeg-board, video, rosco_m68k,
  hackrf-one, PixelWave, SmartSpin2k_Panelized, CyberKeeb2040, four
  bumwings variants and stm32_chiffre_36keys, ErgoSNM.
- 5 whose session KiCad cannot import: SUMEC_MK_IV, poly_kb_molecule
  7x5_wave_left, 5x4_wave and 5x2_shifted, bumwings_v001R64_nano_sd.
- 3 it cannot read: zpn_devboard and the Hub (a NullPointerException in
  `applyBoardSpecificOptimizations`, as on 2026-10-09: no session) and
  OnBoard keyboar_ (the DSN is not read).
- The 0xCB pcb-panel, whose KiCad check times out on both sides.

Ours has no result on two: katia (the harness's 2700 s timeout) and the
pcb-panel.

**Where Freerouting is better** (fewer KiCad-unconnected items; it passes
none of the four):

- **Business-Cards Batch_1** (a panel): ours 106/204, KiCad 441 unconnected,
  no counting error but 261 clearance findings the strict rule counts (the
  designer's board has clearance findings at KiCad's cap); Freerouting 307
  unconnected with 158 counting errors (34 shorting_items, 96 copper edge
  clearance, 8 invalid_outline, 16 starved thermals, 4 hole_to_hole).
- **katia**: ours has no result (the harness's 2700 s timeout on x1367 and
  again on x1371 alone); Freerouting 90 unconnected and 126 hole_clearance,
  in 2643 s.
- **jiran-ble-lite**: ours 104/105, KiCad 20 unconnected, no error (20 GND
  pads on the far half of a split keyboard joined by a frame no track
  crosses); Freerouting 2 unconnected and 23 clearance errors in 75 s.
  Main since x1434 (net ties and apart trees) routes it to KiCad's 1, the
  designer's own.
- **SNSP-CPU-01**: ours 301/363, KiCad 161 unconnected, one starved thermal
  as the strict rule's only finding; Freerouting 151 unconnected with 194
  solder mask bridges, 12 clearance and 3 starved thermals.

Per board (FR open: Freerouting's own count of unrouted connections from
its log, "(wall)" where its job timeout stopped it; "-": no result; routed
CUT: the harness's timeout):

| Board | ours routed | ours KiCad open | ours errors | ours strict | ours wall s | ours CPU s | FR open (its count) | FR KiCad open | FR errors | FR strict | FR wall s | FR CPU s |
| --- | ---: | ---: | ---: | :---: | ---: | ---: | ---: | ---: | ---: | :---: | ---: | ---: |
| 0xCB-1337__1337-v4.0 | 47/47 | 0 | 0 | pass | 100.1 | 100.3 | 1 | 16 | 13 | no | 86.7 | 156.8 |
| 0xCB-1337__panel | 149/188 | 65 | 0 | no | 2355.1 | 2921.2 | 49 | 113 | 20 | no | 954.6 | 2105.1 |
| 0xCB-1337__pcb | 85/85 | 0 | 0 | pass | 405.0 | 729.0 | 1 | 26 | 139 | no | 163.5 | 372.4 |
| 0xCB-1337__pcb-panel | 263/340 | - | - | - | 1306.9 | 673.9 | 125 (wall) | - | - | - | 2215.4 | 2862.6 |
| 0xCB-Static__0xcb-static | 74/74 | 0 | 0 | pass | 31.0 | 31.3 | 197 | 0 | 0 | pass | 40.4 | 123.1 |
| jetson-nano-baseboard__jetson-nano-baseboard | 325/340 | 19 | 0 | no | 820.7 | 1546.2 | 677 (wall) | 499 | 21 | no | 836.5 | 1891.7 |
| explorer__Explorer | 148/150 | 2 | 0 | no | 380.8 | 637.4 | 237 (wall) | 238 | 1 | no | 412.1 | 749.8 |
| eurorack-pmod__eurorack-pmod-pcb | 128/133 | 4 | 0 | no | 465.2 | 1163.3 | 33 (wall) | - | - | - | 768.6 | 1190.6 |
| Qfwfq__qfwfq | 64/64 | 0 | 0 | no | 290.3 | 296.8 | 1 | 15 | 0 | no | 261.1 | 537.8 |
| bit-preserve__coco2 | 173/173 | 0 | 0 | pass | 657.1 | 495.9 | 43 (wall) | 83 | 10 | no | 666.0 | 1520.0 |
| Sumec-MiniSumo__SUMEC_MK_IV | 66/69 | 5 | 0 | no | 974.1 | 1902.8 | 44 | - | - | - | 120.6 | 292.6 |
| d20_pcb__d20_pcb | 18/19 | 1 | 0 | no | 294.2 | 502.2 | 62 | 2 | 152 | no | 58.4 | 98.1 |
| spell_tome__spell_tome_bottom | 18/18 | 0 | 0 | pass | 8.5 | 8.4 | 43 | 1 | 4 | no | 21.6 | 42.0 |
| Sisu_SSE-9__Sisu_SSE-9 | 179/192 | 15 | 0 | no | 792.7 | 1608.2 | 79 (wall) | - | - | - | 1094.7 | 2149.8 |
| laptop__keyboard | 132/136 | 4 | 0 | no | 674.7 | 1197.5 | 27 (wall) | 42 | 12 | no | 684.2 | 1684.0 |
| laptop__motherboard | 232/236 | 4 | 0 | no | 1279.4 | 2055.4 | 6 (wall) | - | - | - | 1580.9 | 3601.0 |
| laptop__power | 110/115 | 5 | 0 | no | 328.2 | 885.3 | 6 (wall) | 50 | 155 | no | 347.5 | 694.8 |
| Business-Cards__Batch_1 | 106/204 | 441 | 0 | no | 810.4 | 1664.9 | 234 (wall) | 307 | 158 | no | 826.2 | 1889.9 |
| Business-Cards__USB_Cable_Tester__PCB | 54/54 | 0 | 0 | pass | 121.6 | 122.1 | 4 (wall) | 23 | 132 | no | 150.0 | 282.0 |
| Business-Cards__USB_Keypad | 37/39 | 2 | 0 | no | 381.4 | 670.3 | 2 | 27 | 23 | no | 81.2 | 181.3 |
| Business-Cards__WLED_Matrix | 99/104 | 11 | 2 | no | 583.0 | 1046.4 | 9 | 75 | 34 | no | 318.3 | 711.5 |
| MyKiCad__framework_mobo_lefthalf | 71/71 | 0 | 0 | pass | 266.2 | 704.4 | 55 (wall) | 63 | 4 | no | 283.7 | 534.6 |
| MyKiCad__protoesp | 67/104 | 58 | 1 | no | 1180.3 | 1805.1 | 90 | 115 | 4 | no | 767.7 | 1684.2 |
| MyKiCad__vaio_re | 262/267 | 4 | 6 | no | 999.7 | 1559.1 | 44 (wall) | - | - | - | 1301.5 | 2613.6 |
| MyKiCad__zpn_devboard | 166/167 | 1 | 0 | no | 823.6 | 1479.4 | - | - | - | - | 4.3 | 4.5 |
| mackerel-68k__mackerel-08-v1 | 96/96 | 0 | 0 | pass | 467.0 | 587.3 | 6 (wall) | 56 | 15 | no | 483.1 | 872.7 |
| mackerel-68k__mackerel-10-v1 | 155/155 | 0 | 0 | pass | 662.2 | 921.2 | 2 (wall) | 121 | 4 | no | 670.8 | 1569.9 |
| mackerel-68k__mackerel-30-proto | 217/217 | 0 | 2 | no | 616.1 | 870.4 | 68 (wall) | 198 | 586 | no | 624.4 | 1501.7 |
| SmartSpin2k__SmartSpin2k_Panelized | 550/585 | 45 | 0 | no | 730.3 | 723.6 | 948 (wall) | - | - | - | 1031.9 | 1924.3 |
| eternal-keypad__eternal-keypad | 65/65 | 0 | 0 | no | 95.6 | 82.9 | 14 (wall) | 19 | 0 | no | 114.2 | 220.2 |
| urchin__main | 68/68 | 0 | 0 | pass | 21.0 | 24.2 | 243 (wall) | 107 | 3 | no | 36.6 | 83.3 |
| atat-1800__ATAT1800 | 253/278 | 30 | 0 | no | 1880.8 | 2218.9 | 38 (wall) | 91 | 27 | no | 1912.4 | 4138.4 |
| osprey__osprey_rev_a | 99/103 | 3 | 1 | no | 2164.8 | 2280.9 | 23 (wall) | 50 | 26 | no | 2179.1 | 4730.6 |
| OpenAirScope__OpenAirScope | 179/184 | 5 | 0 | no | 550.9 | 1221.8 | 6 | 54 | 0 | no | 482.1 | 1032.3 |
| glasgow__glasgow__revC3 | 124/226 | 180 | 0 | no | 883.3 | 1247.7 | 385 (wall) | 390 | 2 | no | 890.0 | 2017.2 |
| ULK__ULK | 68/70 | 2 | 0 | no | 1668.0 | 2022.1 | 3 | 13 | 41 | no | 43.9 | 99.8 |
| ULK__ULK_sl_PG1316s | 68/70 | 3 | 0 | no | 1783.6 | 2382.3 | 5 | 7 | 9 | no | 90.5 | 186.4 |
| hackrf__hackrf-one | 319/319 | 0 | 0 | pass | 284.2 | 423.7 | 726 (wall) | - | - | - | 586.8 | 823.6 |
| OnBoard__E-Fidget-Lite | 20/20 | 0 | 0 | pass | 27.0 | 26.5 | 63 | 2 | 0 | no | 35.9 | 92.1 |
| OnBoard__keyboar_ | 102/102 | 0 | 0 | no | 540.0 | 359.2 | - | - | - | - | 301.7 | 294.0 |
| OnBoard__koeg-board-pcb | 150/175 | 33 | 23 | no | 1630.5 | 2168.0 | 73 (wall) | - | - | - | 1931.7 | 4068.4 |
| OnBoard__krishveercard | 43/43 | 0 | 0 | no | 617.0 | 1658.9 | 86 | 86 | 0 | no | 78.7 | 200.0 |
| OnBoard__MotionCubeViewAllForces | 20/20 | 0 | 0 | pass | 83.1 | 130.8 | 9 | 12 | 8 | no | 86.9 | 184.1 |
| OnBoard__PixelWave | 343/343 | 0 | 0 | pass | 359.3 | 350.7 | 688 (wall) | - | - | - | 663.7 | 962.9 |
| OnBoard__woagboard | 84/91 | 9 | 0 | no | 1410.6 | 2711.4 | 7 | 49 | 0 | no | 793.9 | 2011.4 |
| slab-pcb__interchange-pcb-right | 83/83 | 0 | 0 | no | 983.1 | 795.9 | 16 | 112 | 99 | no | 882.3 | 1957.4 |
| slab-pcb__slab-pcb-left | 75/76 | 1 | 0 | no | 1839.7 | 2228.6 | 9 | 107 | 0 | no | 1136.6 | 2126.3 |
| ecad-viewer__video | 267/371 | 254 | 0 | no | 1368.9 | 1450.3 | 311 (wall) | - | - | - | 1670.8 | 3431.6 |
| silkscreen-pcb__silkscreen_pcb | 113/113 | 0 | 0 | pass | 149.1 | 149.9 | 39 (wall) | 64 | 40 | no | 168.0 | 385.3 |
| katia__katia | - CUT | - | - | - | 2701.0 | 2805.0 | 27 | 90 | 126 | no | 2643.2 | 5879.3 |
| ISS-PCB__BAGEL-MK1 | 138/138 | 0 | 0 | pass | 174.6 | 231.7 | 334 (wall) | - | - | - | 477.5 | 686.7 |
| ISS-PCB__BAGEL-MK1.1-Avocado | 142/144 | 2 | 0 | no | 558.9 | 1295.8 | 242 (wall) | - | - | - | 861.6 | 1592.2 |
| ISS-PCB__MIDAS-MK1.1__MIDAS-MK1.1-revA | 151/156 | 5 | 0 | no | 598.7 | 1429.7 | 15 (wall) | - | - | - | 900.6 | 1727.1 |
| ISS-PCB__MIDAS-MK1__MIDAS-MK1-revA | 151/159 | 8 | 9 | no | 670.0 | 1281.3 | 36 (wall) | 63 | 12 | no | 679.2 | 1611.8 |
| ISS-PCB__MIDAS-MK2 | 156/156 | 0 | 0 | pass | 242.6 | 299.6 | 65 (wall) | - | - | - | 547.8 | 769.5 |
| ISS-PCB__MIDAS-MK2.1__MIDAS-MK2.1-revA | 154/154 | 0 | 0 | pass | 602.4 | 1194.8 | 8 | 46 | 1 | no | 541.6 | 1235.8 |
| ligra__ligra_back | 33/33 | 0 | 0 | pass | 39.1 | 29.5 | 77 | 5 | 0 | no | 26.3 | 68.0 |
| jiran-ble-lite__jiran-ble-lite | 104/105 | 20 | 0 | no | 1318.8 | 1357.0 | 2 | 2 | 23 | no | 74.9 | 196.2 |
| little-red-rover__little_red_rover | 64/64 | 0 | 0 | pass | 317.8 | 553.9 | 11 | 30 | 6 | no | 191.0 | 400.3 |
| M8SBC-486__homebrew_486 | 190/190 | 0 | 0 | pass | 536.4 | 975.4 | 460 (wall) | 488 | 0 | no | 586.1 | 606.0 |
| PCB-Modular-Multi-Protocol-Hub__Hub | 144/159 | 22 | 0 | no | 971.2 | 1945.3 | - | - | - | - | 4.3 | 4.6 |
| Neotron-Pico__neotron-pico | 176/190 | 24 | 0 | no | 2426.6 | 2872.0 | 97 | 103 | 2 | no | 800.8 | 1700.6 |
| Quanta75__Quanta75_BareRP2040_JLCPCBAoptimized | 156/172 | 18 | 1 | no | 1772.8 | 2006.5 | 59 | 76 | 12 | no | 1762.5 | 3409.9 |
| Quanta75__Quanta75_RP2040Stamp_JLCPCBAoptimized | 162/162 | 0 | 0 | pass | 694.6 | 529.8 | 1 | 30 | 9 | no | 172.7 | 402.0 |
| DSP-ADAU1452__DSP-ADAU1452 | 156/190 | 61 | 4 | no | 917.2 | 1578.4 | 152 (wall) | 212 | 61 | no | 924.3 | 2164.1 |
| OpenMuscle-FlexGrid__OM-60-Flex | 21/21 | 0 | 0 | pass | 120.6 | 147.8 | 141 | 0 | 0 | pass | 54.9 | 129.1 |
| OpenMuscle-FlexGrid__OM-FlexGrid-Flex__OM-FlexGrid-Flex | 19/19 | 0 | 0 | pass | 281.3 | 726.1 | 3 | 6 | 0 | no | 171.3 | 373.5 |
| OpenESC-20x20__4in1-mini | 126/152 | 43 | 0 | no | 508.5 | 1197.4 | 347 (wall) | - | - | - | 811.3 | 676.3 |
| OpenESC-30x30__4in1 | 91/152 | 111 | 0 | no | 407.7 | 701.6 | 412 (wall) | - | - | - | 710.1 | 1001.7 |
| OpenESC-30x30__4in1-panel | 47/184 | 485 | 220 | no | 975.2 | 2051.6 | 881 (wall) | - | - | - | 1276.6 | 2403.1 |
| OpenFC-Lite__OpenFC | 82/82 | 0 | 0 | pass | 279.7 | 840.6 | 7 (wall) | 13 | 84 | no | 295.6 | 578.5 |
| OpenRX__OpenRX-panel-rev2 | 42/127 | 229 | 0 | no | 519.5 | 905.6 | 526 (wall) | 499 | 125 | no | 555.7 | 552.9 |
| link__link | 245/251 | 6 | 0 | no | 1770.2 | 2081.4 | 294 (wall) | 420 | 191 | no | 1786.8 | 3460.4 |
| rosco_m68k__rosco_m68k__kicad | 148/148 | 0 | 0 | pass | 597.0 | 728.2 | 21 (wall) | - | - | - | 898.6 | 1698.4 |
| OSHW-reCamera-Series__reCamera_S101_v1.1 | 55/56 | 4 | 0 | no | 116.1 | 310.0 | 56 (wall) | 88 | 0 | no | 136.5 | 216.8 |
| ergo-snm-keyboard__ErgoSNM_keyboard | 58/58 | 0 | 0 | pass | 270.4 | 245.1 | 48 (wall) | - | - | - | 573.2 | 800.0 |
| Telemetry__TelemetryOnboard | 108/108 | 0 | 0 | no | 364.8 | 960.3 | 14 (wall) | 28 | 10 | no | 385.4 | 740.5 |
| capybully_keyboard__capybully | 32/32 | 0 | 0 | pass | 730.2 | 811.1 | 125 (wall) | 57 | 0 | no | 755.4 | 1776.4 |
| le_chiffre_keyboard_stm32__stm32_chiffre_36keys | 76/76 | 0 | 0 | pass | 417.0 | 458.6 | 9 (wall) | - | - | - | 719.5 | 1061.6 |
| le_chiffre_keyboard_stm32__stm32_hotswap_chiffre | 75/75 | 0 | 0 | pass | 454.0 | 486.4 | 6 | 25 | 0 | no | 335.4 | 624.3 |
| MegaDrive__MegaDrive | 243/298 | 168 | 0 | no | 2102.8 | 1533.2 | 193 (wall) | 251 | 35 | no | 2123.5 | 4479.8 |
| nonSNES__SNSP-CPU-01 | 301/363 | 161 | 0 | no | 1632.3 | 1786.1 | 125 (wall) | 151 | 209 | no | 1641.1 | 3695.1 |
| nonSNES__SNSP-CPU-1CHIP | 212/212 | 0 | 0 | pass | 599.9 | 460.7 | 2 | 70 | 22 | no | 497.9 | 1155.5 |
| MokyaLora__MokyaLora | 261/262 | 0 | 0 | pass | 840.6 | 1492.5 | 178 (wall) | 221 | 0 | no | 849.7 | 1998.3 |
| PolyKybd__poly_corne_split42_left | 240/247 | 7 | 0 | no | 1387.5 | 2224.0 | 212 (wall) | - | - | - | 1689.6 | 3479.0 |
| PolyKybd__poly_corne_split42_right | 404/420 | 22 | 0 | no | 1878.8 | 2295.6 | 725 (wall) | 499 | 2 | no | 1899.0 | 4016.8 |
| PolyKybd__poly_kb_molecule_4x4 | 1/28 | 499 | 16 | no | 1300.0 | 2406.9 | 554 (wall) | 499 | 60 | no | 1308.7 | 2960.5 |
| PolyKybd__poly_kb_molecule_4x5 | 0/28 | 499 | 101 | no | 984.1 | 2522.8 | 732 (wall) | 499 | 156 | no | 993.0 | 2248.6 |
| PolyKybd__poly_kb_molecule_5x2_shifted | 1/28 | 409 | 32 | no | 1464.8 | 1857.8 | 301 (wall) | - | - | - | 1470.3 | 3316.4 |
| PolyKybd__poly_kb_molecule_5x4_wave | 2/28 | 499 | 0 | no | 643.4 | 933.6 | 486 (wall) | - | - | - | 648.4 | 1452.8 |
| PolyKybd__poly_kb_molecule_7x5_wave_left | 3/28 | 499 | 0 | no | 926.7 | 1299.8 | 784 (wall) | - | - | - | 932.5 | 2099.1 |
| PolyKybd__poly_kybd_split72_left | 393/420 | 37 | 0 | no | 1677.1 | 2433.3 | 671 (wall) | 499 | 1 | no | 1691.4 | 3840.4 |
| PolyKybd__poly_kybd_split72_right | 405/420 | 24 | 0 | no | 1927.8 | 2388.5 | 747 (wall) | 499 | 0 | no | 1960.2 | 4083.8 |
| CharlieBoard__Blue_Line | 26/26 | 0 | 0 | pass | 11.0 | 11.1 | 72 | 0 | 0 | pass | 20.3 | 52.9 |
| computie__k30-SBC | 173/173 | 0 | 0 | no | 645.6 | 814.2 | 1 | 79 | 8 | no | 532.7 | 1238.2 |
| bumwings-kbd__bumwings_v001 | 72/72 | 0 | 0 | pass | 155.6 | 155.6 | 61 (wall) | - | - | - | 458.5 | 650.3 |
| bumwings-kbd__bumwings_v001_core | 98/98 | 0 | 0 | pass | 592.2 | 706.7 | 36 (wall) | 31 | 0 | no | 602.3 | 1428.3 |
| bumwings-kbd__bumwings_v001_xiao | 81/81 | 0 | 0 | pass | 556.3 | 437.3 | 62 (wall) | - | - | - | 858.3 | 1466.1 |
| bumwings-kbd__bumwings_v001_xiao_s | 78/78 | 0 | 0 | pass | 130.6 | 138.5 | 103 (wall) | 106 | 1 | no | 155.8 | 349.4 |
| bumwings-kbd__bumwings_v001R55_rp2040zero_sd | 70/70 | 0 | 0 | pass | 84.6 | 85.4 | 52 (wall) | - | - | - | 387.7 | 489.1 |
| bumwings-kbd__bumwings_v001R55_xiao_sd | 75/75 | 0 | 0 | pass | 163.6 | 163.8 | 74 (wall) | - | - | - | 468.5 | 668.8 |
| bumwings-kbd__bumwings_v001R64_nano_sd | 85/85 | 0 | 0 | pass | 72.5 | 74.7 | 100 (wall) | - | - | - | 82.1 | 176.2 |
| bumwings-kbd__bumwings_v001R64_rp2040zero_sd | 81/81 | 0 | 0 | pass | 100.1 | 89.9 | 79 (wall) | 79 | 1 | no | 118.0 | 221.1 |
| bumwings-kbd__bumwings_v001R64_xiao_sd | 89/89 | 0 | 0 | pass | 127.1 | 127.8 | 130 (wall) | 132 | 2 | no | 147.5 | 312.9 |
| rp2040-motor-controller__RP2040_base | 147/186 | 50 | 0 | no | 617.4 | 1150.0 | 149 (wall) | 162 | 0 | no | 624.7 | 1407.6 |
| keyboards__ghoul | 142/148 | 6 | 1 | no | 2109.3 | 2740.9 | 7 | 121 | 100 | no | 850.0 | 1906.7 |
| sbc_allwinner_a13__module | 208/223 | 23 | 3 | no | 1023.8 | 1808.4 | 195 (wall) | 256 | 112 | no | 1033.8 | 2328.8 |
| Castor_and_Pollux__mainboard | 126/126 | 0 | 0 | pass | 567.9 | 804.3 | 1 | 41 | 29 | no | 98.7 | 258.8 |
| CyberKeeb2040__MainBoard | 118/118 | 0 | 0 | pass | 350.7 | 378.5 | 99 (wall) | - | - | - | 655.5 | 1013.5 |

**Speed: ten boards, 1 and 16 threads.** 0xcb-static, ligra_back,
1337-v4.0, USB_Cable_Tester, silkscreen_pcb, MIDAS-MK2, little_red_rover,
Castor_and_Pollux mainboard, mackerel-08-v1 and coco2; ours at 1 and 16
threads (`PCB_MAKER_NO_CACHE=1`, one board at a time under the tier
lock, `build/fr2-speed/x1367-t<n>-<board>`), Freerouting at `-mt 1` and
`-mt 16` with a 1500 s job timeout, one board at a time without the lock
(`build/fr2-speed/cmp1-freerouting-mt<n>`), admitted by the memory ledger
(`--min-free-gb 10`; its JVM reserved 4.5 GB there through a wrapper
around `freeroute`). The machine was shared with other agents' runs
throughout; Freerouting's walls below leave out the time it waited for
the ledger. Each cell: wall / CPU seconds, then
KiCad's unconnected items / counting errors ("pass": the strict rule).

| board | ours t1: wall / CPU s | open/errors | ours t16: wall / CPU s | open/errors | FR -mt 1: wall / CPU s | open/errors | FR -mt 16: wall / CPU s | open/errors |
|---|---:|---|---:|---|---:|---|---:|---|
| 0xcb-static | 37 / 34 | 0/0 pass | 40 / 36 | 0/0 pass | 23 / 78 | 0/0 pass | 17 / 60 | 0/0 pass |
| ligra_back | 42 / 36 | 0/0 pass | 34 / 35 | 0/0 pass | 12 / 40 | 5/0 | 14 / 42 | 5/0 |
| 1337-v4.0 | 112 / 114 | 0/0 pass | 123 / 126 | 0/0 pass | 25 / 85 | 16/13 | 27 / 91 | 16/13 |
| PCB | 113 / 116 | 0/0 pass | 136 / 139 | 0/0 pass | 86 / 228 | 23/132 | 118 / 297 | 23/132 |
| silkscreen_pcb | 174 / 173 | 0/0 pass | 188 / 180 | 0/0 pass | 280 / 650 | 59/17 | 374 / 760 | 59/17 |
| MIDAS-MK2 | 409 / 487 | 0/0 pass | 392 / 485 | 0/0 pass | 138 / 328 | 32/1 | 207 / 436 | 32/1 |
| little_red_rover | 226 / 533 | 0/0 pass | 229 / 581 | 0/0 pass | 98 / 250 | 30/6 | 91 / 226 | 30/6 |
| mainboard | 776 / 1131 | 0/0 pass | 664 / 1023 | 0/0 pass | 116 / 282 | 41/29 | 95 / 220 | 41/29 |
| mackerel-08-v1 | 374 / 473 | 0/0 pass | 455 / 563 | 0/0 pass | 257 / 604 | 53/16 | 177 / 426 | 53/16 |
| coco2 | 360 / 352 | 0/0 pass | 278 / 284 | 0/0 pass | 1802 / 3616 | timeout | 1802 / 3772 | timeout |

Ours strict-passes all ten at both thread counts; Freerouting passes
0xcb-static only, leaves 5 to 59 KiCad-unconnected items on eight, and
on coco2 does not stop within the 1500 s job timeout plus 300 s at either
thread count (no result). It stops by itself early: its wall is a median
0.53 times ours at 1 thread and 0.43 times at 16, while its CPU is a
median 1.20 and 1.05 times ours (the JVM's own threads: 2.0 to 3.4 CPU
seconds per wall second at `-mt 1`). Neither router gets faster with 16
threads here: Freerouting's `-mt 16` over `-mt 1` is a median x1.05 CPU
and x1.03 wall with identical results on every board, ours at 16 threads
over 1 is x1.05 CPU and x1.05 wall with the same verdicts (the route
mode's budget is work, not wall). Our runs saw a load of 15 to 39, Freerouting's 9 to 36.

Logs: `build/fr2-route-final.log` (ours), `build/fr2-freerouting-x1367.log`
and `.err`, the table and summary in `build/fr2-compare.txt`
(`freerouting_route_compare.py --markdown`), the speed runs in
`build/fr2-speed/`.

## Freerouting against our cold route, x738 (2026-10-15)

Same setup as the 2026-10-09 round: the designer's placement with every
track, via and copper zone removed, 1500 s wall each, Freerouting 2.2.4
headless through `benchmarks/github/freerouting.py` (now with `-Xmx4g`; an
A/B on hackrf showed the limit costs no CPU, and no board hit it), both
results judged by KiCad's DRC with `--severity-all` against the same
baseline. Our side is pcb-maker-x738 on all 109 boards. Boards ran side by
side (4 jobs, at most 2 Freerouting JVMs and 2 routes at a time), with the
load recorded per row. Freerouting timeouts at a load above 45 were rerun,
and eight boards Freerouting had finished last round but timed out on here
were rerun once Freerouting-only (four then finished).

| | x738 round | x136/x155 round (2026-10-09) |
|---|---|---|
| Freerouting finished / timed out / failed | 67 / 36 / 6 | 70 / 34 / 5 |
| fewer unconnected: pcb-maker / Freerouting | 47 / 6 | 49 / 6 |
| fewer DRC errors: pcb-maker / Freerouting | 33 / 1 | 33 / 0 |
| verdict (unconnected, then errors): ours / FR / tie | 53 / 6 / 8 | 56 / 6 / 8 |
| both finished: unconnected FR / ours | 873 / 184 | 913 / 206 |
| both finished: DRC errors FR / ours | 1857 / 8 | 2041 / 15 |
| both finished: clean boards FR / ours | 8 / 51 | 8 / 50 |
| both finished: vias FR / ours | 5708 / 6785 | 6575 / 7205 |
| both finished: length FR / ours | 409.5 / 409.2 m | 461.4 / 460.4 m |
| both finished: CPU FR / ours | 62688 s / 40885 s | not recorded |
| clean boards over all 109, FR / ours | 8 / 62 | 8 / 60 |
| our unconnected over all 109 | 4785 | 4577 |

Freerouting finished 67, timed out 36, failed 6 (keyboar_, formerly a
"timeout", is a DSN its parser cannot read: a part number with a backtick
loops it forever). On the 67 boards both finish, pcb-maker has fewer
unconnected on 47 and Freerouting on 6; length is equal and we use 19 %
more vias, at 0.65 times Freerouting's CPU seconds (ours including its own
KiCad checks; Freerouting burns about 2 CPU seconds per wall second even
with `-mt 1`). No board flipped sides; the verdict changes are boards where
Freerouting has a result in one round and not in the other (0xCB pcb-panel,
video, osprey, bumwings core: timeouts now; eurorack-pmod: finished now).
Freerouting's wall time on one board varies by more than two times between
runs (hackrf's first pass took 386 s in one run and more than 900 s in two
others), so its timeouts near the limit are noisy.

Against the x136/x155 round our cold route is better on 16 boards, the same
on 82 and worse on 11 (4577 to 4785 unconnected): SNSP-CPU-01 55 to 178
(all GND; the router reports 362 of 363 connections routed while KiCad
finds 178 unconnected terminals) and MegaDrive 95 to 137 repeat exactly
when run alone, and katia (84 to 183) is cut by the 1800 s wall in both of
its runs at a 14.9 GB peak. Better rows include vaio 26 to 8, the A13
module 22 to 9, DSP 7 to 0 and mackerel-30's 13 errors to 0.

The runner changes (CPU, load and tier per row, the jobs file,
`--resume`/`--redo`, `--java-heap`, stall detection, `--severity-all`)
landed in ef980b2; the DRC-cache round made both DRCs go through
`pcb-maker drc-kicad-board` when the binary has it (047f6ff; x738 has not,
so this round used kicad-cli on both sides). Logs:
`build/github-freerouting-x738.log`, reruns `-fr2.log` and `-katia.log`,
table `-compare.txt`. Reproduce the summary with
`freerouting_compare.py build/github-freerouting-x738.log build/github-freerouting-x738.log build/github-freerouting-x738-fr2.log --ours-splice build/github-freerouting-x738-katia.log --previous-ours build/github-ours-cold-all.log --previous-freerouting build/github-freerouting.log build/github-freerouting-2.log build/github-freerouting-3.log`.

## Layout sweep v16 (2026-10-15)

`build/github-layout-v16.log`, binary x862 (main 068de8b: determinism,
fill rounds two and three, CLI exit codes, DRC cache, placer fixes, ladder
reservation, clean-up work accounting, inner spokes, hatched zones in the
tracks rung; it predates the first-route budget, the hole-clearance fix,
the router overhead round and the second narrow attempt), tasks v6, 5 jobs
beside other agents (load 16 to 35 for most rows), 20876 s wall.

| | v15 (x723) | v16 (x862) |
|---|---|---|
| pass | 45 of 109 | 44 of 109 |
| unconnected (all rows) | 3868 | 3527 |
| new DRC errors | 289 | 82 |
| vias | 25759 | 25335 |
| CPU s (unpaired) | 189814 | 145292 |
| rows cut by the wall clock | not reported | 17 |
| rows without a result | 9 | 10 |

Better 13, same 58, worse 13, 13 cut rows left out of that count. Large
moves: video 499 to 300 open, SNSP-CPU-01 246 to 168, RP2040_base 173 to
96, 0xCB panel 210 to 86 (cut), woagboard 26 open and 26 errors to 8 and
1 (cut), SUMEC 28 to 18, Hub 35 to 30; katia now finishes (104 open);
molecule 5x4_wave places legally. Worse: ULK and poly_kybd_split72_left
now time out, the 0xCB pcb-panel ends on kicad-cli's 900 s zone refill,
SmartSpin2k_Panelized and OpenESC 4in1-panel stop with parts the placer
refuses, Quanta75 Bare 123 to 132 open (errors 73 to 6), split42_right 46
to 61 (cut). The 17 cut rows at modest load say some phase's wall runs
well ahead of its counted work; triage in
[changelog.md](changelog.md#triage-of-v16-2026-10-15-x931--x932). Compare with
`python3 benchmarks/agent-tasks/compare.py build/github-layout-v15.log build/github-layout-v16.log`.

## Layout sweep v17 (2026-10-16)

`build/github-layout-v17.log`, binary x1170 (main 54eff39, the one-time
rustfmt commit; code = everything up to the KiCad check charge: first-route
budget, hole clearance, router overhead and search core and stamp rounds,
pour-work recalibration, second narrow attempt, zone seams, connection
width, min connection, mask bridges and mask openings, placement room,
panel fixes, orientation search, order independence, calibration 3.15e6,
check work), tasks v7, tier default 8 jobs x 2 threads, 65959 s wall.

| | v16 (x862) | v17 (x1170) |
|---|---|---|
| pass | 44 of 109 | 48 of 109 |
| unconnected (all rows) | 3527 | 2708 |
| new DRC errors | 82 | 38 |
| vias | 25335 | 30601 |
| CPU s (unpaired) | 145292 | 101279 |
| rows cut by the wall clock | 17 | 2 |
| rows without a result | 10 | 7 |

Better 25, same 50, worse 8 (14 cut rows and 1 on other inputs left out).
ULK, SmartSpin2k_Panelized and poly_kybd_split72_left now finish (68/70,
553/585 and 394/420); MegaDrive finishes but is cut at 163 open. The 0xCB
pcb-panel still ends on kicad-cli's 900 s zone refill, the four molecule
panels still leave a few back connectors unplaced (designer stacking at
0.03 mm), OpenESC 4in1-panel is crowded, and MIDAS-MK1 is a new refusal
(6 parts without a legal place: triage assigned). Vias rose 21 percent
with the 1.57x larger work budget. The two cuts ran at loads 1 and 66.
Compare with
`python3 benchmarks/agent-tasks/compare.py build/github-layout-v16.log build/github-layout-v17.log`.

## Layout sweep v18 (2026-10-21)

**Matched-budget designer tier (2026-10-22):** with the designer's placement routed at the layout's median final-ladder share (budget scale 0.695, x1404, strict rule 3), 59 of 109 pass (51 strict) and the gap over the 99 target boards is placement 49 %, routing 51 % (layout open 1973, designer-route open 1016), against 71/29 at the full 1500 s. The placement share above is therefore an upper bound; routing at the layout's budget is as large a lever as placement. Log: `build/designer-x1404-matched.log`.

`build/github-layout-v18.log`, binary x1315 (main 35456a1: project rules
at scale, runtime calibration, the ladder's work bound and fault
isolation; 6f10989 differs only in doc comments), tasks v7 (the committed
`benchmarks/agent-tasks/tasks/v7`, the same files as v17's
`build/github-tasks-v7`: tasks.json sha256 c8f95caf5089 and every
constraint file byte-identical; compare.py marks no board INPUT), with
`--quality`, at the pinned work rates (every row: `work_rate` 3.15e6,
`placer_work_rate` 150e6, `work_rate_source` PCB_WORK_RATE; the measured
router rate had a median of 2.3e6, 0.99e6 to 3.3e6, the placer's 6.5e7,
work over wall 0.77). The sweep ran in three parts, all on x1315, beside
other agents' runs (load median 20, peaks of 77):

- `build/github-layout-v18a.log`: 6 jobs x 2 threads, 20 GB of memory
  kept free, about 4.9 hours, 41 boards; memory admission held it to two
  or three boards at a time (hackrf waited 3120 s), so it was stopped (its
  boards in flight lost);
- `build/github-layout-v18b.log`: the other 68 boards at 10 x 2 with 6 GB
  kept free (`--splice-from` the first part), 8834 s wall; four rows cut
  by the wall guard (10 layouts and the designer tier side by side);
- `build/github-layout-v18c.log`: the four cut boards again at 2 x 2
  (`--splice-from` the second part), 5211 s wall. 0xCB panel and Quanta75
  Bare finished uncut with the same rows as when cut; katia and Batch_1
  were cut again (below). This log is `build/github-layout-v18.log`; the
  output directories are `build/github-layout-v18`, `-v18b` and `-v18c`.

| | v17 (x1170) | v18 (x1315) |
|---|---|---|
| pass, per-type rule | 48 of 109 | 46 of 109 |
| pass, strict rule | not recorded | **42 of 109** |
| unconnected (all rows) | 2708 | 2712 |
| new DRC errors (per-type rule) | 38 | 50 |
| vias | 30601 | 31044 |
| length | 779 m | 788 m |
| work s | 80276 (102 rows) | 86094 (103 rows) |
| CPU s (unpaired; v18 oversubscribed) | 101279 | 152463 |
| rows cut by the wall clock | 2 | 2 |
| rows without a result | 7 | 6 |
| vias against the designer's (median) | x0.70 | x0.71 |
| length against the designer's (median) | x1.03 | x1.03 |
| open over connections (median, mean) | 0.011, 0.287 | 0.009, 0.284 |
| ground pour cut, ours less the designer's (median) | +0 mm (95 boards, 1 lost the pour) | +0 mm (96 boards, 1 lost the pour) |

(v17's pour cut from `compare.py --score build/bin/pcb-maker-x1315`.)
Better 5, same 87, worse 7 (3 rows cut in either run left out). Better:
Glasgow 165 to 160 open, DSP-ADAU1452 68 to 64, split72_left 39 to 36,
video 201 to 199, BAGEL-MK1.1-Avocado 2 to 1. Worse: split42_right 19 to
26, OpenRX panel 219 to 226, the A13 module 8 to 12, RP2040_base 44 to
47, Sisu 13 to 15, split72_right 28 to 30, laptop power 3 to 4.
mackerel-30-proto and rosco lose their pass to starved thermals (6 and 2
under the strict rule); MIDAS-MK1 places now (153/159, 6 open; a refusal
in v17). Under the strict rule four plain passes fail: Qfwfq
(hole_clearance) and krishveercard (clearance), each a type the
designer's stripped board has at KiCad's cap of 199, which the strict rule
cannot place and counts whole, and coco2 and k30-SBC, one starved thermal
each.

Boards that fail the strict rule on findings of one type alone (nothing
unconnected): starved_thermal on four (mackerel-30-proto 6, rosco 2,
coco2 1, k30-SBC 1), and the two capped types above (Qfwfq, krishveercard,
which the route on the designer's placement fails the same way).

Cut rows, at 2 x 2 as at 10 x 2 and with the same rows both times: katia
(157/162, 75 open; cut at 3040 s of wall with 1155 of its 1500 s of work
done, work over wall 0.38) and Batch_1 (115/204, 415 open, a panel of
business cards; cut at 3296 s with 1280 s of work done, 0.39).

### Placement or routing: the designer tier (x1315)

`build/designer-x1315b.log` (`--tier designer` through the lock, 8 x 2,
30179 s wall: `build/designer-x1315.log`; its two timeouts again at 2 x 2
with `--splice-from`): every board routed by the same binary on the
designer's own placement, 1500 s of work, the layout's pass rules, the
same pinned rates (106 rows; the three without a result carry none).
**50 of 109 pass the strict rule** (58 the per-type rule; 49 under strict
rule 3 of 2026-10-22, re-judged without rerouting: USB_Keypad's one
connection_width on its GND pour had been subtracted against the
designer's 45 on that pour, all at (0, 0); the layout sweep stays at 42,
its strict errors 1536 -> 1543); vias x0.47 of
the designer's, length x0.99, the ground pours cut 38 mm less than the
designer's (medians). katia and Batch_1 hit the harness's 3300 s in both
runs: katia's ladder runs one router at a time for memory (7.3 GB each)
and had done its probes, four attempts and a seed round (best 279 open
terminals by the router's count); Batch_1 was in its finer pitch's seed
round at 1241 open terminals. The 0xCB pcb-panel has no result on either
side (kicad-cli times out after 900 s; on the designer's placement the
router itself completes 340/340). The count is not the x738 cold route's 62 clean
(2026-10-15): that round removed every copper zone and judged against the
stripped source's own DRC; the designer tier keeps the large pours, as
the layout does, and applies the layout's two pass rules.

`compare.py --gap build/github-layout-v18.log build/designer-x1315b.log`:
over the 101 boards with both results, layout open 2222, designer-route
open 1153, placement gap +1069, routing gap 1153 (48 % and 52 %). But
723 of those 1153 are on boards that are not targets (docs/handover.md,
"Not targets": the molecule 5x4_wave panel 499, the OpenRX panel 213,
SUMEC 11, krishveercard 0; the other molecule panels and the OpenESC 4in1
panel have no layout result). Over the 97 target boards with both
results: **layout open 1483, designer-route open 430: placement gap
+1053 (71 %), routing gap 430 (29 %)**; the placement gap is +1064 on the boards where ours is
worse and -11 where it is better. 22 % of the layout's open items are on
supply nets, 34 % of the designer route's.

By the strict pass on both placements (100 target boards):

- **Placement** (the designer's placement passes, ours does not), 13
  (12 under strict rule 3: USB_Keypad fails on both placements):
  video (ours 199 open), OpenESC-30x30 4in1 (111), 0xCB panel (65,
  starved_thermal 1), OpenAirScope (5), laptop power (4), ULK_sl_PG1316s
  (3), ULK (2), USB_Keypad (2), Explorer (2), zpn_devboard (1),
  slab-pcb-left (1), mackerel-30-proto (0 open, starved_thermal 6), rosco
  (0 open, starved_thermal 2).
- **Neither passes**, 44. Both complete, both failing on findings: 4
  (Qfwfq, coco2, eternal-keypad, k30-SBC). The designer route leaves as
  many open or more, so routing alone: 14, 162 open ours against 172
  (RP2040_base 47 / 47, ATAT1800 29 / 30, neotron-pico 24 / 24, jiran
  20 / 20, Sisu 15 / 18, ghoul 6 / 6, MIDAS-MK1.1 5 / 6, reCamera 4 / 4,
  laptop keyboard 4 / 4, interchange-pcb-right 4 / 7, capybully 1 / 3,
  BAGEL-MK1.1-Avocado, d20 and silkscreen_pcb 1 / 1). Ours leaves more,
  so both: 24 (ours / designer route): Glasgow 160 / 69, MegaDrive
  150 / 71, SNSP-CPU-01 136 / 23, DSP-ADAU1452 64 / 3, protoesp 58 / 18,
  4in1-mini 46 / 4, SmartSpin2k 39 / 0 (the designer route fails only on
  the capped invalid_outline), split72_left 36 / 4, Quanta75 Bare
  34 / 13, split72_right 30 / 5, split42_right 26 / 5, koeg 25 / 1,
  jetson 24 / 2, Hub 22 / 16, WLED_Matrix 12 / 1, the A13 module 12 / 7,
  link 11 / 1, split42_left 11 / 1, woagboard 8 / 1, eurorack-pmod
  6 / 1, MIDAS-MK1 6 / 5, laptop motherboard 4 / 2, vaio_re 4 / 3,
  osprey 2 / 1. No designer result: katia (ours 75) and Batch_1 (415).
- **Ours passes, the designer's placement does not**, 5: OpenFC (the
  designer route 1 open), framework_mobo_lefthalf, hackrf-one and
  keyboar_ (one starved thermal each), Quanta75 RP2040Stamp (clearance 2,
  shorting_items 1, solder_mask_bridge 1).
- Both pass: 37. No layout result: the 0xCB pcb-panel.

Two router gaps that are modelling, not search: **net ties** are not
modelled (no code mentions them): every one of neotron-pico's 24 open
connections on both placements is a pad of one of its twelve net-tie
footprints (NT901 to NT912, an EEPROM address line to +3V3 or GND), and
so are all six of ghoul's and 10 to 13 of protoesp's (40 of the designer
route's 430 on target boards). And **pads of one footprint with the same
number** (jiran-ble-lite's USB shield pads S1 and the stacked GND pads,
interchange-pcb-right, capybully: 15 more), which KiCad counts as
separate items to join.

**What it says.** Placement is the larger lever: on the target boards our
placement adds 1053 open connections to the 430 our router leaves on the
designer's placement, 13 boards pass on the designer's placement and not
on ours against 5 the other way, and 24 of the 44 boards that fail on
both leave more open on ours. The placement gap is concentrated: video
199, SNSP-CPU-01 113, the OpenESC 4in1 111, Glasgow 91, MegaDrive 79, the
0xCB panel 65, DSP-ADAU1452 61, 4in1-mini 42, protoesp 40, SmartSpin2k 39
and split72_left 32 hold 872 of the 1064 (82 %), dense or bus-heavy
boards that the designer's placement makes routable for this router. The
routing gap is spread over 38 target boards, more than half of it on
five (MegaDrive 71, Glasgow 69, RP2040_base 47, ATAT1800 30, neotron-pico 24), and 55 of
the 430 are the two modelling gaps above. Earlier placement knobs that
counted pins (routing room, routing demand) did not move the dense
boards (docs/handover.md, open problem 4), RUDY predicts a final result
at a Spearman of 0.30 and the 75 s probe at 0.55 (2026-10-10): what the
placement lacks is a routability measure that is both fast and good. A
coarse global route (tiles with per-layer capacity, negotiated, seconds a
board) is that measure, and is the corridor plan the detailed router
could follow later. Cheaper first: net ties (neotron-pico's and ghoul's
designer routes would be complete) and the starved thermals that alone
fail four boards (open problem 5).

## Layout sweep v19 (2026-10-22)

The first full measurement on one binary after the packages of
2026-10-21 and 22 (net ties and solid thermals, no vias in SMD pads,
all pour work charged, probe overhead, strict rule 3, the free dimension
and the layout budget fixes, the harness review fixes).
`build/github-layout-v19.log`, binary x1539 (main af9d32a, sha256
ded67b2895c1), tasks v7 (`benchmarks/agent-tasks/tasks/v7/tasks.json`,
sha256 c8f95caf5089, the same list as v18), with `--quality`, at the pinned
work rates (`work_rate` 3.15e6, `placer_work_rate` 150e6, PCB_WORK_RATE;
the measured router rate had a median of 2.2e6, work over wall 0.87),
10 jobs x 2 threads through the tier lock with 14 GB kept free, in one
part: 18399 s wall, largest peak 10.4 GB, beside other agents' runs (load
median 12, peaks of 39). v18 is on x1315, so the comparison below is
between two binaries. v18's rows were judged under strict rule 1 (four
under rule 3); `rejudge.py` judged them again under rule 3
(`build/github-layout-v18-rule3.log`) and no row changed: 46 / 42.

| | v18 (x1315, rule 3) | v19 (x1539) |
|---|---|---|
| pass, per-type rule | 46 of 109 | **52 of 109** |
| pass, strict rule (rule 3) | 42 of 109 | **49 of 109** |
| unconnected (all rows) | 2712 | 1827 |
| unconnected, the 99 boards with a result in both | 1948 | 1827 |
| new DRC errors (per-type rule) | 50 | 43 |
| new DRC errors (strict rule) | 1543 | 1326 |
| vias | 31044 (103 rows) | 31642 (99 rows) |
| length | 788 m | 789 m |
| work s | 86094 (103 rows) | 85552 (99 rows) |
| CPU s (unpaired) | 152463 | 134523 |
| rows cut by the wall clock | 2 (katia, Batch_1) | **0** |
| rows killed (exit -9 or 137) | 0 | 0 |
| rows without a result | 6 | 10 |
| our vias in small SMD pads (score-kicad-board) | 1299 on 74 of 103 boards | **0 on all 99** |
| vias against the designer's (median) | x0.71 | x0.74 |
| length against the designer's (median) | x1.03 | x1.08 |
| open over connections (median, mean) | 0.009, 0.284 | 0.000, 0.090 |
| final ladder's share of the budget (median) | 0.695 (62 rows) | 0.640 (58 rows) |

The unconnected total of all rows falls by 885, but 764 of that is four
boards that have no result in v19 (below); on the 99 boards with a result
in both it falls by 121. compare.py: better 28, same 53, worse 16 (the two
v18 rows cut left out). The medians are over different board sets (103
and 99). **The ground pour cut is not comparable**: x1539's
score-kicad-board gives the designer's own boards no cut on 79 of 94 (x1315:
19 of 96; on hackrf-one's source the same file scores 76.95 mm under x1315
and 0 under x1539, our v19 result 2294.93 mm under both), so the median
"+2142 mm against the designer's" is a scorer change, not a routing one; our
raw median is 2869 mm in v18 and 3166 mm in v19. The designer's own vias in
pads also count differently (3211 on 65 boards under x1315, 2218 on 53
under x1539: the exposed-pad and same-number pad grouping).

Better, the largest: DSP-ADAU1452 64 to 30 open, neotron-pico
24 to 0 (every open connection was a net-tie pad), SNSP-CPU-01 136 to 114,
jiran 20 to 1, jetson 24 to 5, koeg 25 to 9, the 0xCB panel 65 to 53,
split72_left 36 to 24, the A13 module 12 to 0, SUMEC 14 to 4. Uncut now:
katia 75 to 38 and Batch_1 415 to 234. Worse, the largest: vaio_re 4 to 89, MegaDrive
150 to 222, protoesp 58 to 106, 4in1-mini 46 to 81, Glasgow 160 to 191,
the OpenESC 4in1 111 to 127, Sisu 15 to 27, split42_right 26 to 31,
ATAT1800 29 to 34, eurorack-pmod 6 to 11.

Pass status that changed (strict rule):

- **Gained, 9.** Starved thermals gone (solid thermals): mackerel-30-proto
  (6), rosco's `kicad` (2), coco2 (1) and k30-SBC (1); copper to the edge
  gone: silkscreen_pcb (copper_edge_clearance 6); the last open
  connection routed: zpn_devboard, BAGEL-MK1.1-Avocado, Explorer (2) and
  slab-pcb-left. interchange-pcb-right gains the per-type pass only (4
  open to 0; copper_edge_clearance 24, which the route on the designer's
  placement has as well).
- **Lost, 2.** OM-FlexGrid-Flex has no result: "1 parts without a legal
  place (J1)", J1 held by an edge, region or overhang constraint (the
  held-parts and free dimension changes; it passed in v18). homebrew_486
  leaves one connection open: its GND zone on F.Cu, B.Cu and In2.Cu in two
  islands (KiCad's unconnected item is the zone against itself at
  (170.9, 23.4)).

No vias in SMD pads anywhere, so no board changed by that rule's findings.
Strict rule 3 changed no v18 row.

Rows without a result, 10 (v18: 6): the 0xCB pcb-panel (kicad-cli's DRC
times out after 900 s, as in v18), and nine refusals with parts without a
legal place: the five molecule panels (4x4, 4x5, 5x2_shifted,
7x5_wave_left as in v18 with more parts each; 5x4_wave new, "J3, J4,
silkscreen mark", 499 open in v18), the OpenESC 4in1 panel (42 parts, 216
in v18), and three that had a result in v18: the OpenRX panel (81 parts;
226 open in v18), SmartSpin2k_Panelized (12 parts; 39 open) and
OM-FlexGrid-Flex (J1; a pass). All but OM-FlexGrid-Flex are panels and
not targets (docs/handover.md, "Not targets"). A drawn item ending beyond
the outline is infeasible since the free dimension fixes (e4cc2cc), which
is the likely cause of the "silkscreen mark" refusals.

Boards that fail the strict rule with nothing open: Qfwfq
(hole_clearance) and krishveercard (clearance), each at KiCad's cap of
199 (per-type passes); eternal-keypad (shorting_items 1, hole_clearance
199); neotron-pico (hole_clearance 2, the designer route 1); the A13
module (items_not_allowed 2; the designer route items_not_allowed 1 and 1 open);
interchange-pcb-right (copper_edge_clearance 24, the designer route the
same).

### Placement or routing on one binary (matched designer tier, x1539)

`build/designer-x1539-matched.log`: every board routed by x1539 on the
designer's own placement at `--budget-scale 0.695` (1042.5 s of work, v18's
median final-ladder share), 8 x 1 through the lock, 14 GB kept free; in
two parts on the same binary (`build/designer-x1539-matched-a.log`, 11
rows, stopped and spliced in with `--splice-from`; the rest 19173 s wall),
no row cut or killed, one without a result (the 0xCB pcb-panel, kicad-cli
times out). **54 of 109 pass the strict rule** (61 the per-type rule);
vias x0.49 of the designer's, length x0.99 (medians). Against the matched
x1404 run (`build/designer-x1404-matched.log`): better 17, same 90, worse
1, unconnected 4098 -> 3594, passes 59 -> 61 (strict 51 -> 54).

v19's own final-ladder share is lower than v18's: median 0.640 over the 58
rows where it ran (0.624 over the 39 of them left open; compare.py --gap
says 0.638 over 56 target rows). So at 0.695 the designer route had some
8 % more routing work than v19's final ladder, and the routing gap below is
a slight underestimate (the placement share a slight overestimate).

`compare.py --gap build/github-layout-v19.log build/designer-x1539-matched.log --targets`:
over the 97 target boards with both results, **layout open 1823,
designer-route open 747: placement gap +1076 (59 %), routing gap 747
(41 %)**; ours is worse on 33 boards, the same on 58, better on 6. No
layout result: the 0xCB pcb-panel, OM-FlexGrid-Flex and
SmartSpin2k_Panelized. (x1404 matched against v18: 1973 and 1016, 49 / 51.)

By the strict pass on both placements (100 target boards):

- **Placement** (the designer's placement passes, ours does not), 9:
  video (ours 199 open), the OpenESC 4in1 (127), the 0xCB panel (53), ULK
  (4), ULK_sl_PG1316s (2), laptop power (2), homebrew_486 (1), capybully
  (1), OM-FlexGrid-Flex (no legal place for J1).
- **Ours passes, the designer's placement does not**, 4: Quanta75
  RP2040Stamp (the designer route: clearance 2, shorting_items 2,
  solder_mask_bridge 1), k30-SBC (one starved thermal), OpenFC (1 open),
  BAGEL-MK1.1-Avocado (2 open).
- Both pass: 45. Neither passes: 42, among them the largest placement
  gaps (ours / designer route open): Glasgow 191 / 50, MegaDrive 222 / 109,
  protoesp 106 / 1, 4in1-mini 81 / 2, vaio_re 89 / 10, SNSP-CPU-01
  114 / 46, split42_right 31 / 3, DSP-ADAU1452 30 / 4; and routing alone
  (the designer route leaves as many): RP2040_base 47 / 48, ATAT1800
  34 / 34, Sisu 27 / 26, OpenAirScope 4 / 4, reCamera 4 / 4.

**Where a fix gains most** (target boards by the open connections our
placement adds, with what KiCad reports on our result): video 199 open
(267/371 routed, no other finding; the designer route 0), Glasgow 191
(113/226 routed; designer route 50), the OpenESC 4in1 127 (88/152;
designer route 0), MegaDrive 222 (224/298; designer route 109, so +113),
protoesp 106 (65/104, solder_mask_bridge at KiCad's cap and clearance 2;
designer route 1 open and the same capped mask bridges). For passes, the
nearest are the placement boards above with one to four open (ULK,
ULK_sl_PG1316s, laptop power, homebrew_486, capybully) and OM-FlexGrid-Flex's
refusal.

## Expansion intake: 5 834 boards routed on the designer's placement (2026-10-18)

`benchmarks/expansion/` (README there) harvested 5 834 native KiCad boards
from 1 146 repositories (plus 1 225 Eagle and Altium conversions, kept
apart) and routed each on the designer's placement with tracks, vias and
small zones stripped, at a 30-second ladder budget on the be4b012 binary,
one search thread at nice 19 on a loaded machine; 2 635 of them again at
300 s; 2 168 placement variants with features for placement data. The
analysis tables are in `benchmarks/expansion/analysis/` (the raw data,
54 GB, stays on disk under `data/`).

| | boards | share |
|---|---|---|
| native-clean within 30 s of routing | 2 309 | 39.6 % |
| native-clean at any time on the quick run | 2 626 | 45.0 % |
| designer-relative pass (no finding type above the designer's, nothing unconnected) | 3 896 | 66.8 % |
| complete by the router's count | 4 367 | 74.9 % |
| complete at 300 s | 4 528 | 77.6 % |
| boards with a native-clean source and their own rules: clean within 30 s | 71.7 % of 1 995 | |

Median routing time of the clean boards 3.4 s (p90 34.5 s). Caveats: 59 %
of the designer boards have DRC findings of their own; 6.9 % of boards use
inferred rules; 75 % are 2-layer; resource limits (199, a 3 GiB cap the
ladder's memory sizing cannot see) and wall cuts (279, overrun work beyond
the ladder budget) are harness and sizing effects, not routing failures
(since 2026-10-21 the ladder sizes its routers by the process's limits and
keeps its work within its budget: docs/changelog.md, "ladder memory and
work bound").

What blocks boards at scale, ranked by boards a single new finding type
blocks: KiCad disagreeing with the router's "complete" (119: stacked
same-number F/B pads, tracks ending at a pad on the other side, pour to pad
after refill; 192 boards complete by the router's count with KiCad
unconnected items in all, classified and fixed on 2026-10-21: docs/changelog.md,
"KiCad connectivity agreement"), starved thermals (103), track_width (82, almost all inferred
rules against KiCad's 0.2 mm default), clearance from `.kicad_dru` and
netclass rules (16), copper edge clearance (15), mask bridges (8). 1 297 of
the 1 725 "DRC failures" fail only on the designer's own findings. Of the
63 model errors, 9 are ours (degenerate polygons, `gr_bbox` pads, a missing
roundrect ratio, `/A15` vs `A15` net identity, one unexplained "no routable
connections"); 54 are unloadable or netless sources. More budget rarely
helps: of 1 258 boards incomplete at 30 s with a 300 s run, 161 became
complete. A 46-board "expansion" tier spanning the hardness axis is
proposed in `analysis/expansion-tier.json`.

The tier is built (2026-10-21): `run.py --tier expansion` routes the 46
boards on the designer's placement at 30 s of work (`--budget-scale 10`
for 300 s), fetching each at its pinned commit with SHA-256 checks and a
licence rule (benchmarks/agent-tasks/README.md, "The expansion tier").
x1211, 8 jobs x 1 thread: 14/46 pass, 45 of 46 boards in the intake's
outcome class, every stripped board identical and every route the wall
guard cut in neither run identical (32 boards); 13 rows are cut by the
wall guard at that load.

## KiCadRoutingTools against x1367 (2026-10-21)

**Complete: all 109 boards judged** (measured 2026-10-21 to 2026-10-23),
and the ten-board timing with ours at 1 and 16 threads. Our side is
pcb-maker-x1367 throughout (`build/fr2-route-final.log`, the rows of the
Freerouting refresh above); main has moved on since (x1573 and later, see
[changelog.md](changelog.md)), so these are not main's numbers.

[KiCadRoutingTools](https://github.com/drandyhaas/KiCadRoutingTools) (MIT,
commit cddf4cd of 2026-10-21, version 0.23.1) is the autorouter a Hacker
News commenter found "routed more sensibly" than Freerouting: Python around
a Rust A* core, an octilinear 0.1 mm grid on every copper layer, rip-up and
reroute, plane pours, fanout, a CLI and a KiCad plugin. It ran headless
through its CLI router (`py_router/route.py <board> <result> --nets '*'
--no-fix-drc-settings --escalation board`, through
`benchmarks/github/kicadroutingtools.py`; setup in
[benchmarks/github/README.md](../benchmarks/github/README.md#kicadroutingtools-kicadroutingtoolspy)).
It routed the placements of the Freerouting refresh
(`build/github-layout-v17/<task>/layout/placed`, the 109 tasks of v7) with
the designer's pours kept; route.py treats them as planes.

Both results are judged alike:
- KiCad's DRC through `drc-kicad-board`, with the zones refilled.
- The pass: unconnected items and copper errors beyond the designer's, per
  type.
- The strict rule.

Two flags keep the judging ours. By default route.py loosens the written
project's Board Setup floors to whatever it routed, and lets a failing net
drop below the board's minimums to the fab tier's. The result is judged
with the placement's project files; route.py rewrote none of them.

route.py has no time limit and no thread setting. Its search runs on one
thread: the Rust core has no threads, and the BLAS pools were pinned to
one. A killed run leaves no board unless it had already written one: it
writes the board after its rescue pass, before its plane finalize and
reconciliation. So each board ran to completion or to a cap, and each row
records its wall time next to ours. The cap:
- The 32 largest boards ran first, eight at a time, with three, later two,
  times our wall, at most 3600 s (`build/krt-route-x1367`, `-b`).
- The other 67 boards: our wall, at least 120 s, two routers at a time
  (`build/krt-route-x1367-c`; their KiCad checks waited for the tier lock
  and finished on 2026-10-22).
- The ten timing boards ran to completion, at most 3600 s
  (`build/krt-timing`); their rows replace the capped ones in the table.

Our walls were measured at a load of 12 to 50 (8 x 2 beside other agents);
its runs saw a load of 2 to 46 (at the start of a board), most of the capped ones 2 to 15, which
favours it on "within our wall".

**Summary** (`benchmarks/github/kicadroutingtools_compare.py
build/krt-route-x1367 build/krt-route-x1367-b build/krt-route-x1367-c
build/krt-timing --ours build/fr2-route-final.log`):

| | pcb-maker x1367 | KiCadRoutingTools cddf4cd |
| --- | ---: | ---: |
| boards with a KiCad-checked result (of 109) | 107 | 66 (36 finished, 30 cut after writing its board) |
| cut before writing a board | | 43 |
| pass (designer-subtracted per type) | 51 | 7 (3 within our wall) |
| strict pass | 44 | 5 (1 within our wall) |
| better on KiCad's unconnected, then errors | 93 | 6 (10 ties) |
| 66 boards both have: KiCad unconnected | 1 778 | 3 296 |
| same: copper errors (pass rule) | 69 | 3 661 |
| same: vias | 10 985 | 25 200 |
| same: track length | 320.2 m | 319.1 m |
| finished within our wall | | 24 |
| wall over ours, all 109, a cut run at its cap (a lower bound) | | median x1.00, at least ours on 85 |
| CPU / wall over ours where it finished (36) | | median x0.60 / x0.75 |

Every board it passes ours passes too, under the strict rule as well:
spell_tome_bottom, Blue_Line, urchin, ligra_back, bumwings_v001_core,
bumwings_v001_xiao and CyberKeeb2040 (its strict passes: not ligra_back
and bumwings_v001_core). It is faster on three of them (bumwings_v001_core
164 s against our 592, bumwings_v001_xiao 268 against 556, ligra_back 30 against 39) and
slower on four (CyberKeeb2040 1870 s against 351). The 36 boards it
finishes are the ones it finds easy: there its median CPU is 0.60 times
ours, because ours spends its budget on the boards it cannot close and
KiCadRoutingTools stops when its passes are done, complete or not (29 of
the 36 do not pass). Its copper errors are mostly hole_clearance (1 037),
solder_mask_bridge (919), annular_width (392), via_diameter (367),
starved_thermal (240), copper_edge_clearance (232) and drill_out_of_range
(225): vias at or below the board's floors, often in a pad. Peak memory
was 0.2 to 5.3 GB a board.

**Where KiCadRoutingTools is better** (fewer KiCad-unconnected items,
then errors; it passes none of the six), with what KiCad reports on ours:
- **d20_pcb**: KiCadRoutingTools 18/18 nets, KiCad 0 unconnected but 199
  solder_mask_bridge, in 193 s. Ours 18/19, 1 unconnected, no error, in
  294 s.
- **Business-Cards WLED_Matrix**: KiCadRoutingTools 10 unconnected with 14
  connection_width, 73 hole_clearance, 2 copper_edge_clearance and 4
  dangling items, in 268 s. Ours 11 unconnected and 2
  copper_edge_clearance, in 583 s.
- **jiran-ble-lite**: KiCadRoutingTools 1 unconnected (the designer's own
  count), no error, in 141 s. Ours 20 unconnected (the far half's GND
  pads), no error, in 1319 s; main since x1434 routes it to the
  designer's 1.
- **0xCB-1337 panel**: KiCadRoutingTools 50 unconnected, 84
  solder_mask_bridge and 17 starved_thermal, cut at 3600 s. Ours 149/188,
  65 unconnected, no error, in 2355 s.
- **Neotron-Pico**: KiCadRoutingTools 190/190 nets, KiCad 0 unconnected,
  2 annular_width, 2 hole_clearance and 1 hole_to_hole (its vias at 0.25
  mm, the board floor), 727 vias, in 971 s. Ours 176/190, 24 unconnected,
  no error, 212 vias, in 2427 s (1177 s of work at a load of 25 to 49).
- **glasgow revC3**: KiCadRoutingTools 163 unconnected, cut at 2650 s
  with its board written. Ours 124/226, 180 unconnected, no error, in
  883 s.

Per board (`build/krt-compare.txt`; "nets" is KiCadRoutingTools' own
count; a cut row says how far its log got):

| board | conn. | ours: unconn. / errors / strict | KRT: nets / unconn. / errors / strict | wall s ours / KRT | CPU s ours / KRT |
|---|---:|---|---|---:|---:|
| katia/katia | ? | timed out | cut at 3600 s (main loop 162/162 nets, 82 failed) | 2701 / 3600 | 2805 / 2518 |
| spell_tome/spell_tome_bottom | 18 | 0 / 0 / pass | 18/18 / 0 / 0 / pass | 8 / 28 | 8 / 45 |
| OpenMuscle-FlexGrid/OM-FlexGrid-Flex | 19 | 0 / 0 / pass | cut at 281 s (before the main loop) | 281 / 281 | 726 / 280 |
| d20_pcb/d20_pcb | 19 | 1 / 0 / fail | 18/18 / 0 / 199 / fail | 294 / 193 | 502 / 213 |
| OnBoard/E-Fidget-Lite | 20 | 0 / 0 / pass | 17/19 / 2 / 0 / fail | 27 / 32 | 26 / 52 |
| OnBoard/MotionCubeViewAllForces | 20 | 0 / 0 / pass | 18/20 / 2 / 0 / fail | 83 / 44 | 131 / 62 |
| OpenMuscle-FlexGrid/OM-60-Flex | 21 | 0 / 0 / pass | cut at 121 s (before the main loop) | 121 / 121 | 148 / 120 |
| CharlieBoard/Blue_Line | 26 | 0 / 0 / pass | 26/26 / 0 / 0 / pass | 11 / 60 | 11 / 76 |
| PolyKybd/poly_kb_molecule_4x4 | 28 | 499 / 16 / fail | ? / 499 / 98 / fail (cut at 3552 s, board written) | 1300 / 3552 | 2407 / 3383 |
| PolyKybd/poly_kb_molecule_4x5 | 28 | 499 / 101 / fail | cut at 984 s (before the main loop) | 984 / 985 | 2523 / 979 |
| PolyKybd/poly_kb_molecule_5x2_shifted | 28 | 409 / 32 / fail | ? / 447 / 81 / fail (cut at 1465 s, board written) | 1465 / 1465 | 1858 / 1516 |
| PolyKybd/poly_kb_molecule_5x4_wave | 28 | 499 / 0 / fail | cut at 643 s (main loop 27/27 nets, 2 failed) | 643 / 644 | 934 / 639 |
| PolyKybd/poly_kb_molecule_7x5_wave_left | 28 | 499 / 0 / fail | cut at 927 s (main loop 27/27 nets, 1 failed) | 927 / 927 | 1300 / 921 |
| capybully_keyboard/capybully | 32 | 0 / 0 / pass | ? / 18 / 0 / fail (cut at 730 s, board written) | 730 / 730 | 811 / 751 |
| ligra/ligra_back | 33 | 0 / 0 / pass | 32/32 / 0 / 0 / fail | 39 / 30 | 30 / 48 |
| Business-Cards/USB_Keypad | 39 | 2 / 0 / fail | ? / 6 / 77 / fail (cut at 3600 s, board written) | 381 / 3600 | 670 / 3607 |
| OnBoard/krishveercard | 43 | 0 / 0 / fail | 32/43 / 36 / 0 / fail | 617 / 389 | 1659 / 405 |
| 0xCB-1337/1337-v4.0 | 47 | 0 / 0 / pass | 43/46 / 4 / 22 / fail | 100 / 77 | 100 / 131 |
| Business-Cards/PCB | 54 | 0 / 0 / pass | ? / 2 / 307 / fail (cut at 122 s, board written) | 122 / 122 | 122 / 126 |
| OSHW-reCamera-Series/reCamera_S101_v1.1 | 56 | 4 / 0 / fail | 51/56 / 6 / 39 / fail (cut at 120 s, board written) | 116 / 120 | 310 / 153 |
| ergo-snm-keyboard/ErgoSNM_keyboard | 58 | 0 / 0 / pass | ? / 11 / 7 / fail (cut at 3600 s, board written) | 270 / 3600 | 245 / 3568 |
| Qfwfq/qfwfq | 64 | 0 / 0 / fail | cut at 290 s (main loop 63/63 nets, 4 failed; rescue pass done, 4 recovered) | 290 / 290 | 297 / 298 |
| little-red-rover/little_red_rover | 64 | 0 / 0 / pass | ? / 6 / 0 / fail (cut at 318 s, board written) | 318 / 318 | 554 / 353 |
| eternal-keypad/eternal-keypad | 65 | 0 / 0 / fail | 63/63 / 2 / 1 / fail | 96 / 1037 | 83 / 1082 |
| urchin/main | 68 | 0 / 0 / pass | 68/68 / 0 / 0 / pass | 21 / 90 | 24 / 92 |
| Sumec-MiniSumo/SUMEC_MK_IV | 69 | 5 / 0 / fail | 62/69 / 15 / 77 / fail | 974 / 1830 | 1903 / 1496 |
| ULK/ULK | 70 | 2 / 0 / fail | 67/69 / 2 / 3 / fail | 1668 / 634 | 2022 / 679 |
| ULK/ULK_sl_PG1316s | 70 | 3 / 0 / fail | 66/69 / 3 / 0 / fail | 1784 / 1255 | 2382 / 1198 |
| bumwings-kbd/bumwings_v001R55_rp2040zero_sd | 70 | 0 / 0 / pass | 70/70 / 0 / 1 / fail | 85 / 111 | 85 / 132 |
| MyKiCad/framework_mobo_lefthalf | 71 | 0 / 0 / pass | cut at 266 s (main loop 71/71 nets, 7 failed) | 266 / 266 | 704 / 264 |
| bumwings-kbd/bumwings_v001 | 72 | 0 / 0 / pass | 72/72 / 1 / 0 / fail | 156 / 90 | 156 / 111 |
| 0xCB-Static/0xcb-static | 74 | 0 / 0 / pass | cut at 120 s (before the main loop) | 31 / 120 | 31 / 120 |
| le_chiffre_keyboard_stm32/stm32_hotswap_chi… | 75 | 0 / 0 / pass | ? / 4 / 0 / fail (cut at 454 s, board written) | 454 / 454 | 486 / 520 |
| bumwings-kbd/bumwings_v001R55_xiao_sd | 75 | 0 / 0 / pass | cut at 164 s (before the main loop) | 164 / 164 | 164 / 163 |
| slab-pcb/slab-pcb-left | 76 | 1 / 0 / fail | 68/78 / 18 / 8 / fail | 1840 / 2476 | 2229 / 2381 |
| le_chiffre_keyboard_stm32/stm32_chiffre_36k… | 76 | 0 / 0 / pass | ? / 9 / 0 / fail (cut at 417 s, board written) | 417 / 417 | 459 / 462 |
| bumwings-kbd/bumwings_v001_xiao_s | 78 | 0 / 0 / pass | cut at 131 s (main loop 78/78 nets, 2 failed) | 131 / 131 | 138 / 130 |
| bumwings-kbd/bumwings_v001R64_rp2040zero_sd | 81 | 0 / 0 / pass | cut at 120 s (before the main loop) | 100 / 120 | 90 / 120 |
| bumwings-kbd/bumwings_v001_xiao | 81 | 0 / 0 / pass | 81/81 / 0 / 0 / pass | 556 / 268 | 437 / 290 |
| OpenFC-Lite/OpenFC | 82 | 0 / 0 / pass | 76/82 / 7 / 40 / fail | 280 / 147 | 841 / 170 |
| slab-pcb/interchange-pcb-right | 83 | 0 / 0 / fail | ? / 91 / 0 / fail (cut at 983 s, board written) | 983 / 983 | 796 / 1010 |
| 0xCB-1337/pcb | 85 | 0 / 0 / pass | ? / 2 / 8 / fail (cut at 405 s, board written) | 405 / 405 | 729 / 449 |
| bumwings-kbd/bumwings_v001R64_nano_sd | 85 | 0 / 0 / pass | cut at 120 s (before the main loop) | 72 / 120 | 75 / 120 |
| bumwings-kbd/bumwings_v001R64_xiao_sd | 89 | 0 / 0 / pass | cut at 127 s (before the main loop) | 127 / 127 | 128 / 127 |
| OnBoard/woagboard | 91 | 9 / 0 / fail | 80/91 / 15 / 4 / fail | 1411 / 476 | 2711 / 520 |
| mackerel-68k/mackerel-08-v1 | 96 | 0 / 0 / pass | 96/96 / 0 / 92 / fail | 467 / 339 | 587 / 359 |
| bumwings-kbd/bumwings_v001_core | 98 | 0 / 0 / pass | 98/98 / 0 / 0 / fail | 592 / 164 | 707 / 190 |
| OnBoard/keyboar_ | 102 | 0 / 0 / fail | 102/103 / 2 / 2 / fail | 540 / 162 | 359 / 208 |
| osprey/osprey_rev_a | 103 | 3 / 1 / fail | 90/103 / 18 / 3 / fail | 2165 / 1547 | 2281 / 1464 |
| Business-Cards/WLED_Matrix | 104 | 11 / 2 / fail | 98/104 / 10 / 93 / fail | 583 / 268 | 1046 / 378 |
| MyKiCad/protoesp | 104 | 58 / 1 / fail | ? / 79 / 4 / fail (cut at 1180 s, board written) | 1180 / 1180 | 1805 / 1184 |
| jiran-ble-lite/jiran-ble-lite | 105 | 20 / 0 / fail | 104/105 / 1 / 0 / fail | 1319 / 141 | 1357 / 148 |
| Telemetry/TelemetryOnboard | 108 | 0 / 0 / fail | ? / 16 / 6 / fail (cut at 365 s, board written) | 365 / 365 | 960 / 428 |
| silkscreen-pcb/silkscreen_pcb | 113 | 0 / 0 / pass | cut at 149 s (main loop 112/112 nets, 13 failed) | 149 / 149 | 150 / 148 |
| laptop/power | 115 | 5 / 0 / fail | 107/114 / 15 / 260 / fail | 328 / 326 | 885 / 377 |
| CyberKeeb2040/MainBoard | 118 | 0 / 0 / pass | 118/118 / 0 / 0 / pass | 351 / 1870 | 378 / 1795 |
| Castor_and_Pollux/mainboard | 126 | 0 / 0 / pass | 120/126 / 6 / 60 / fail | 568 / 417 | 804 / 479 |
| OpenRX/OpenRX-panel-rev2 | 127 | 229 / 0 / fail | ? / 388 / 537 / fail (cut at 520 s, board written) | 520 / 520 | 906 / 520 |
| eurorack-pmod/eurorack-pmod-pcb | 133 | 4 / 0 / fail | ? / 15 / 251 / fail (cut at 465 s, board written) | 465 / 465 | 1163 / 528 |
| laptop/keyboard | 136 | 4 / 0 / fail | 123/136 / 14 / 2 / fail | 675 / 643 | 1198 / 661 |
| ISS-PCB/BAGEL-MK1 | 138 | 0 / 0 / pass | 124/138 / 22 / 197 / fail | 175 / 1021 | 232 / 1163 |
| ISS-PCB/BAGEL-MK1.1-Avocado | 144 | 2 / 0 / fail | ? / 55 / 55 / fail (cut at 559 s, board written) | 559 / 559 | 1296 / 576 |
| rosco_m68k/kicad | 148 | 0 / 0 / pass | cut at 597 s (main loop 148/148 nets, 10 failed) | 597 / 597 | 728 / 594 |
| keyboards/ghoul | 148 | 6 / 1 / fail | ? / 83 / 23 / fail (cut at 3600 s, board written) | 2109 / 3601 | 2741 / 3588 |
| explorer/Explorer | 150 | 2 / 0 / fail | cut at 381 s (main loop 150/150 nets, 18 failed) | 381 / 381 | 637 / 378 |
| OpenESC-20x20/4in1-mini | 152 | 43 / 0 / fail | 118/152 / 57 / 105 / fail | 508 / 335 | 1197 / 371 |
| OpenESC-30x30/4in1 | 152 | 111 / 0 / fail | cut at 408 s (main loop 152/152 nets, 14 failed; rescue pass done, 3 recovered) | 408 / 408 | 702 / 404 |
| ISS-PCB/MIDAS-MK2.1-revA | 154 | 0 / 0 / pass | cut at 602 s (main loop 153/153 nets, 10 failed) | 602 / 603 | 1195 / 598 |
| mackerel-68k/mackerel-10-v1 | 155 | 0 / 0 / pass | ? / 2 / 0 / fail (cut at 662 s, board written) | 662 / 662 | 921 / 692 |
| ISS-PCB/MIDAS-MK1.1-revA | 156 | 5 / 0 / fail | 149/156 / 8 / 0 / fail | 599 / 598 | 1430 / 675 |
| ISS-PCB/MIDAS-MK2 | 156 | 0 / 0 / pass | 125/156 / 53 / 23 / fail | 243 / 1044 | 300 / 1122 |
| ISS-PCB/MIDAS-MK1-revA | 159 | 8 / 9 / fail | 143/159 / 22 / 9 / fail | 670 / 592 | 1281 / 642 |
| PCB-Modular-Multi-Protocol-Hub/Hub | 159 | 22 / 0 / fail | ? / 30 / 21 / fail (cut at 2914 s, board written) | 971 / 2914 | 1945 / 2409 |
| Quanta75/Quanta75_RP2040Stamp_JLCPCBAoptimi… | 162 | 0 / 0 / pass | ? / 22 / 26 / fail (cut at 695 s, board written) | 695 / 695 | 530 / 698 |
| MyKiCad/zpn_devboard | 167 | 1 / 0 / fail | ? / 22 / 205 / fail (cut at 2471 s, board written) | 824 / 2471 | 1479 / 2374 |
| Quanta75/Quanta75_BareRP2040_JLCPCBAoptimiz… | 172 | 18 / 1 / fail | cut at 3600 s (main loop 172/172 nets, 31 failed) | 1773 / 3600 | 2006 / 2512 |
| bit-preserve/coco2 | 173 | 0 / 0 / pass | cut at 657 s (main loop 172/172 nets, 2 failed) | 657 / 657 | 496 / 653 |
| computie/k30-SBC | 173 | 0 / 0 / fail | cut at 646 s (before the main loop) | 646 / 646 | 814 / 642 |
| OnBoard/koeg-board-pcb | 175 | 33 / 23 / fail | cut at 3600 s (main loop 175/175 nets, 9 failed) | 1630 / 3600 | 2168 / 2531 |
| OpenESC-30x30/4in1-panel | 184 | 485 / 220 / fail | cut at 2926 s (main loop 177/177 nets, 37 failed) | 975 / 2926 | 2052 / 2423 |
| OpenAirScope/OpenAirScope | 184 | 5 / 0 / fail | cut at 551 s (main loop 184/184 nets, 27 failed; rescue pass done, 18 recovered) | 551 / 551 | 1222 / 548 |
| rp2040-motor-controller/RP2040_base | 186 | 50 / 0 / fail | ? / 216 / 56 / fail (cut at 617 s, board written) | 617 / 618 | 1150 / 616 |
| 0xCB-1337/panel | 188 | 65 / 0 / fail | ? / 50 / 101 / fail (cut at 3600 s, board written) | 2355 / 3601 | 2921 / 4802 |
| Neotron-Pico/neotron-pico | 190 | 24 / 0 / fail | 190/190 / 0 / 5 / fail | 2427 / 971 | 2872 / 821 |
| M8SBC-486/homebrew_486 | 190 | 0 / 0 / pass | cut at 536 s (main loop 190/190 nets, 16 failed) | 536 / 536 | 975 / 535 |
| DSP-ADAU1452/DSP-ADAU1452 | 190 | 61 / 4 / fail | ? / 210 / 74 / fail (cut at 2752 s, board written) | 917 / 2752 | 1578 / 2398 |
| Sisu_SSE-9/Sisu_SSE-9 | 192 | 15 / 0 / fail | ? / 114 / 187 / fail (cut at 3600 s, board written) | 793 / 3600 | 1608 / 3660 |
| Business-Cards/Batch_1 | 204 | 441 / 0 / fail | cut at 3600 s (main loop 183/183 nets, 28 failed; rescue pass done, 7 recovered) | 810 / 3601 | 1665 / 3536 |
| nonSNES/SNSP-CPU-1CHIP | 212 | 0 / 0 / pass | cut at 600 s (before the main loop) | 600 / 600 | 461 / 596 |
| mackerel-68k/mackerel-30-proto | 217 | 0 / 2 / fail | cut at 616 s (main loop 217/217 nets, 12 failed; rescue pass done, 57 recovered) | 616 / 616 | 870 / 613 |
| sbc_allwinner_a13/module | 223 | 23 / 3 / fail | ? / 218 / 117 / fail (cut at 3071 s, board written) | 1024 / 3072 | 1808 / 2516 |
| glasgow/revC3 | 226 | 180 / 0 / fail | ? / 163 / 0 / fail (cut at 2650 s, board written) | 883 / 2650 | 1248 / 2490 |
| laptop/motherboard | 236 | 4 / 0 / fail | ? / 147 / 155 / fail (cut at 3600 s, board written) | 1279 / 3600 | 2055 / 3112 |
| PolyKybd/poly_corne_split42_left | 247 | 7 / 0 / fail | cut at 3600 s (main loop 247/247 nets, 82 failed) | 1388 / 3601 | 2224 / 2368 |
| link/link | 251 | 6 / 0 / fail | cut at 3600 s (main loop 251/251 nets, 125 failed) | 1770 / 3600 | 2081 / 2388 |
| MokyaLora/MokyaLora | 262 | 0 / 0 / pass | ? / 27 / 0 / fail (cut at 2522 s, board written) | 841 / 2522 | 1492 / 2479 |
| MyKiCad/vaio_re | 267 | 4 / 6 / fail | cut at 2999 s (main loop 266/266 nets, 29 failed) | 1000 / 2999 | 1559 / 2445 |
| atat-1800/ATAT1800 | 278 | 30 / 0 / fail | cut at 3600 s (main loop 278/278 nets, 96 failed; rescue pass done, 80 recovered) | 1881 / 3600 | 2219 / 2326 |
| MegaDrive/MegaDrive | 298 | 168 / 0 / fail | cut at 3600 s (main loop 298/298 nets, 2 failed) | 2103 / 3600 | 1533 / 3519 |
| hackrf/hackrf-one | 319 | 0 / 0 / pass | cut at 284 s (main loop 319/319 nets, 1 failed) | 284 / 284 | 424 / 283 |
| 0xCB-1337/pcb-panel | 340 | no result | cut at 3548 s (main loop 340/340 nets, 51 failed; rescue pass done, 7 recovered) | 1307 / 3548 | 674 / 2872 |
| jetson-nano-baseboard/jetson-nano-baseboard | 340 | 19 / 0 / fail | cut at 2462 s (main loop 340/340 nets, 23 failed) | 821 / 2462 | 1546 / 2337 |
| OnBoard/PixelWave | 343 | 0 / 0 / pass | 340/343 / 3 / 20 / fail | 359 / 964 | 351 / 1019 |
| nonSNES/SNSP-CPU-01 | 363 | 161 / 0 / fail | cut at 1632 s (main loop 363/363 nets, 5 failed) | 1632 / 1633 | 1786 / 1623 |
| ecad-viewer/video | 371 | 254 / 0 / fail | cut at 3600 s (main loop 371/371 nets, 24 failed) | 1369 / 3601 | 1450 / 2395 |
| PolyKybd/poly_corne_split42_right | 420 | 22 / 0 / fail | cut at 3600 s (main loop 420/420 nets, 145 failed) | 1879 / 3600 | 2296 / 2484 |
| PolyKybd/poly_kybd_split72_left | 420 | 37 / 0 / fail | cut at 3600 s (main loop 420/420 nets, 137 failed) | 1677 / 3600 | 2433 / 2474 |
| PolyKybd/poly_kybd_split72_right | 420 | 24 / 0 / fail | cut at 3600 s (main loop 420/420 nets, 148 failed) | 1928 / 3600 | 2388 / 2483 |
| SmartSpin2k/SmartSpin2k_Panelized | 585 | 45 / 0 / fail | cut at 730 s (main loop 585/585 nets, 137 failed) | 730 / 730 | 724 / 726 |

**Timing on ten boards.** These span the sizes: spell_tome_bottom,
USB_Keypad, ErgoSNM, eternal-keypad, mackerel-08, CyberKeeb2040, BAGEL-MK1,
MIDAS-MK2, Sisu and PixelWave. KiCadRoutingTools ran each to completion
(at most 3600 s); it has no thread setting, so 1 and 16 threads are the
same run for it, and its CPU seconds equal its wall to within a few
percent. Ours ran at 1 and 16 threads with `PCB_MAKER_NO_CACHE=1`, one
board at a time beside other agents' runs: at 1 thread under the tier
lock (`build/krt-ours-t1`), at 16 without it, admitted by the memory
ledger (`--min-free-gb 10`, `build/krt-ours-t16`; MIDAS-MK2's and
mackerel-08's 16-thread rows are the Freerouting study's). Each cell: wall /
CPU seconds, then KiCad's unconnected items / copper errors ("pass": the
strict rule).

| board | ours t1: wall / CPU s | open/errors | ours t16: wall / CPU s | open/errors | KRT: wall / CPU s | open/errors | KRT CPU over ours t1 / t16 |
|---|---:|---|---:|---|---:|---|---:|
| spell_tome_bottom | 8 / 8 | 0/0 pass | 8 / 9 | 0/0 pass | 28 / 45 | 0/0 pass | x6.0 / x5.1 |
| USB_Keypad | 195 / 349 | 2/0 | 397 / 734 | 2/0 | 3600 / 3607 | 6/77 (cut at 3600 s) | > x10.3 / > x4.9 |
| ErgoSNM_keyboard | 88 / 89 | 0/0 pass | 153 / 156 | 0/0 pass | 3600 / 3568 | 11/7 (cut at 3600 s) | > x40.0 / > x22.9 |
| eternal-keypad | 31 / 31 | 0/0 | 46 / 48 | 0/0 | 1037 / 1082 | 2/1 | x34.7 / x22.8 |
| mackerel-08-v1 | 207 / 268 | 0/0 pass | 455 / 563 | 0/0 pass | 339 / 359 | 0/92 | x1.3 / x0.6 |
| MainBoard | 226 / 226 | 0/0 pass | 344 / 372 | 0/0 pass | 1870 / 1795 | 0/0 pass | x7.9 / x4.8 |
| BAGEL-MK1 | 193 / 253 | 0/0 pass | 239 / 322 | 0/0 pass | 1021 / 1163 | 22/197 | x4.6 / x3.6 |
| MIDAS-MK2 | 313 / 372 | 0/0 pass | 392 / 485 | 0/0 pass | 1044 / 1122 | 53/23 | x3.0 / x2.3 |
| Sisu_SSE-9 | 696 / 1309 | 15/0 | 623 / 1293 | 15/0 | 3600 / 3660 | 114/187 (cut at 3600 s) | > x2.8 / > x2.8 |
| PixelWave | 551 / 439 | 0/0 pass | 289 / 302 | 0/0 pass | 964 / 1019 | 3/20 | x2.3 / x3.4 |

Over the seven boards it finished, KiCadRoutingTools' CPU is a median 4.6
times ours at 1 thread and 3.6 times ours at 16 threads (wall: 3.5 and 3.3
times); its three cut runs (USB_Keypad, ErgoSNM, Sisu) are at least 2.8
times. The one board it finished on less CPU is mackerel-08 against our
16-thread run (359 against 563 s), with 92 copper errors to our none; ours
is the better result on five of the seven, and both strict-pass the
other two (spell_tome_bottom, CyberKeeb2040). 16 threads did not make ours faster here: half of the
1-thread runs (the small boards) ran at a load of 1 to 3, the 16-thread
runs at 13 to 34, and the route mode's budget is work, not wall.

### What it has that we lack (for the roadmap)

Read from its source at cddf4cd (2026-10-21; paths relative to its
checkout). Size first: about 776 000 lines of Python (py_router 184 000,
py_placer 85 000, py_tools 19 000, the KiCad plugin 21 000, tests and
measurement scripts 398 000, an experimental CP-SAT bus router `awx/`
64 000) around 5 055 lines of Rust; `py_router/route.py` alone is 9 496
lines. The search is single-ended A* in Rust over (x, y, layer) on a
0.1 mm grid (0.05 mm for tap retries, 0.025 mm for rescues), eight
directions, a via a layer change in place, octile heuristic weighted 2.3;
obstacle rasterisation is Python. Rip-up takes up to three blockers from
the failed search's frontier (ranked by count, nearness to the target,
both directions or a soft-cost min-cut probe), with ghost costs on ripped
corridors and PathFinder-style history costs. Nothing runs in parallel:
no threads in the Rust core, one Python process.

**Differential pairs with polarity.** A centreline is routed by a
pose-based A* in Rust (`pose_router.rs`, `dubins.rs`: x, y, eight
headings and the layer; moves straight, a 45-degree turn followed by
`min_radius` straight steps, or a via that keeps the heading; the
heuristic is the Dubins path length at the turning radius), with the
pair's half-width plus clearance as its clearance, a cumulative-turn
limit against loops, vias only straight in and straight out, and both
P/N via sites checked. P and N are offsets of the simplified centreline
(miter-scaled at bends, per-layer pitch). Polarity is the sign of P's
side at both ends; a mismatch is fixed by leaving one end on the other
side ("connector flip") or by swapping the target pads' nets (opt-in by
pattern), applied to the board and, with `--schematic-dir`, to the pin
numbers in the schematic's lib_symbols. Fallbacks: a coupled middle with
single-ended legs, and single-ended routing for electrically short pairs.
GND return vias beside the signal vias are chosen in Rust during the via
expansion. Intra-pair skew is matched with meanders on the shorter leg.
About 15 500 lines (Python core 12 700: route_diff.py 2 961,
diff_pair_routing.py 5 431, diff_pair_loop.py 1 222,
diff_pair_multipoint.py 1 239; Rust 931; GUI 1 630).

**Length and time matching.** After routing; a member's length is track
length plus via barrels (stackup thickness between the joined layers)
plus the pad's die length; the longest member is the target and the
others get trombone meanders (rectangular bumps with 45-degree chamfers
at a pitch of a multiple of the width) on their longest single-layer
straight runs, each bump checked against a clearance index and shrunk
(0.7 times, down to 0.2 mm) or shifted until it fits, iterated to the
tolerance; pairs are meandered on the centreline and regenerated. Groups
are given by pattern or `auto`, a DDR3/DDR4 name parser that forms byte
lanes (DQ, DQS, DM) and command/address groups per interface and
channel. Time matching converts with sqrt(eps_eff)/c per segment (outer
layers (eps_r+1)/2 or Hammerstad's width-dependent value, inner eps_r,
from the stackup). `.kicad_dru` length windows are checked, not routed
to. About 4 300 lines (length_matching.py 4 185).

**Impedance-controlled widths.** `impedance.py` (1 793 lines):
IPC-2141/Hammerstad microstrip with Hammerstad-Jensen thickness
correction, IPC-2141 symmetric and asymmetric stripline, edge-coupled
pairs by an empirical coupling factor, coplanar waveguide over ground by
conformal mapping (elliptic integrals by AGM); the width for a target
impedance by bisection, per layer, from the board's stackup (dielectric
height, eps_r, copper thickness), clamped at the fab floor. The router
then takes a static width per layer and net. `check_impedance.py`
(1 112) audits reference-plane voids and split crossings under the
routed tracks. About 2 900 lines.

**BGA and QFN fanout.** BGA (`bga_fanout/`, 14 600 lines): the default
escapes through channels between ball rows (a 45-degree stub into a
channel, out past the array, layers assigned with collision repair and
rebalance); `underpad` puts a via in each signal pad and runs a small
per-ball A* on an inner layer under the array, deepest balls first;
`dogbone` puts the via in the diagonal gap; rotated and back-side parts
by frame transforms. QFN/QFP (`qfn_fanout/`, 2 700): two-segment stubs
along each pad's axis, optionally a via just past the pad. A Hungarian
assignment (scipy, `target_swap.py` 1 756) swaps swappable target pads
to minimise distance plus crossings. Decoupling capacitors are nudged
and rotated off foreign fanout vias toward same-net balls
(`fanout_clearance.py` 5 060). About 27 700 lines with the GUI.

**Placement optimisation.** `place_optimize.py` runs a zero-temperature
quench from the existing placement (`quench.py` 5 431): nudges within a
displacement radius, rotations, same-footprint swaps; the objective is
the per-net MST airwire length plus a penalty per airwire crossing, a
whitespace halo and an edge margin; legality (courtyards per side,
Edge.Cuts containment, pad clearance, declared floorplan zones) is hard
(`legality.py` 7 301). `place_route_loop.py` (1 594) puts the router in
the loop: route, read the failed nets and their blockers, micro-quench
the small parts on them with those nets weighted, accept only if
failures and effort improve, else revert and widen. It does not place
from scratch except from a declared floorplan intent (`seeder.py`,
`floorplan.py`, 23 000 lines together). py_placer is 85 000 lines; the
quench and the loop about 14 700.

**DRC and connectivity checkers.** `check_drc.py` (6 248) is its own
geometric DRC on the parsed board (clearances between tracks, vias and
pads, holes, board edge, same-net crossings, fab minimums, vias in paste
openings, `.kicad_dru` length windows) with KiCad's netclass and rule
resolution, but no zone fill; `check_connected.py` (2 448) is union-find
connectivity with zones by a raster fill model; `check_weird.py` (1 012)
copper hygiene (dangling ends, redundant loops, stacked duplicates).
KiCad is the oracle: `kicad_oracle.py` (3 523) runs `kicad-cli pcb drc
--refill-zones` on a copy whose severities are all off but
unconnected_items, and routes exactly the links KiCad reports open;
`kicad_exact_fill.py` (1 193) drives pcbnew's zone filler for exact
islands; every route.py ends with a plane finalize (region joins, pad
taps, clean-up) and that oracle pass. About 20 000 lines (24 000 with
the plane repair).

**The AI tab and Claude skills.** The plugin spawns Claude Code (`claude
-p "/<skill> ..." --output-format stream-json --allowedTools
Read,Glob,Grep,Bash,Agent,Task,WebSearch,WebFetch`) or opencode headless,
streams the transcript and reads a final `RESULT=` line back into the
dialog's fields. `/plan-pcb-routing` (a 3 504-line SKILL.md) reads the
board (stackup, parts needing fanout, diff pairs, DDR groups, power and
speed classes) and returns a JSON plan of steps (fanout, cap clean-up,
route_diff, route, route_planes) that the plugin executes in-process
through each tab's own handler; the AI only plans. Plans can be saved,
replayed without a model, built from a recorded command chain
(`make_plan.py`) and run headless (`run_plan.py`). Other skills: power
nets and widths and high-speed nets from datasheets found by web search,
diff pairs by pin function, stackup, plane mappings, failure diagnosis,
post-route review, an end-to-end place-and-route agent, and a corpus
stress test. About 14 300 lines (plugin AI code and plan tools 5 300,
SKILL.md files 6 700, their scripts 1 800).

Logs: `build/krt-route-x1367{,-b,-c}.log`, `build/krt-timing.log`,
`build/krt-ours-t1.log`, `build/krt-ours-t16.log`, `build/krt-chain.log`;
the whole table and summary in `build/krt-compare.txt`. Our timing runs
are a loop of `PCB_MAKER_NO_CACHE=1 run.py build/krt-ours-t<threads>/<task>
--route-from build/github-layout-v17 --only <task> --jobs 1 --threads
<threads> --min-free-gb 10 --binary build/bin/pcb-maker-x1367`.

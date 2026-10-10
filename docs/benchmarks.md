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

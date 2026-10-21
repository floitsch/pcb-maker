# Changelog

What each round of work measured and changed, newest first: the dated
sections of the old hand-over (docs/handover.md up to 2026-10-16), moved
here verbatim. "Above" and "below" in them refer to the old hand-over's
order, oldest first; read "above" as further down this file. The current
state, the benchmarks that count and the open problems are in
[handover.md](handover.md); git log has the commits themselves.

Sections are ordered by the date in their heading, newest first, and
within one date by when they were written. At the end: the sweeps before
v16 and the hand-over's summary of the week up to 2026-10-09 (binary x155).

## Committed on 2026-10-22 (wall guard cuts at small budgets)

A cut row is not the binary's deterministic answer, so cuts must be rare
and explained. In route mode at 30 s, VortacDesign's and FoCTow's
toolchanger panels (intake boards) were cut in every run, idle or not;
ChengminLi's flyback (rules boards), megaduck, and OpenDeck, brickpico
and Wonkle (expansion tier) when the load was 45 to 70; the layout sweep
v18 (10 x 2, load 30 to 40) had four cut rows of 76. Branch
deadline-cuts; snapshots x1370 (on 642e28c), x1371 (rebased on 401ab2f,
with the check model's outline term); x1366 is the before-binary.

**Where the wall clock went.** Each board alone, one thread, the
reference's rates pinned, x1366 then x1371 back to back (load 15 to 25;
"share" is the machine's search rate measured at the start over the
pinned 3.15e6). x1366's split is from its `profile:` lines, x1371's from
the new `wall accounting:` line (thread seconds; routers side by side
add up).

| board | x1366: wall s / work s (share) | cut | where the wall went beyond the work | x1371: wall s / work s (share) | cut |
| --- | --- | --- | --- | --- | --- |
| VortacDesign | 92 / 19.1 (0.77); 166 / 20.4 (0.64) earlier | yes, every run | manual spokes 46 s in the continuation's finish, 58-69 s in each rung after it (uncharged); probes' pour refresh (34.6 s of pour work beside 18.3 of search, all probes) | 91 / 31.8 (0.38) | no |
| FoCTow (the same panel) | 118 / 19.1 (0.37) | yes | as Vortac: manual spokes 59 s in the verbose router alone | 77 / 31.8 (0.42) | no |
| megaduck | 93 / 46.2 (0.42); 104 / 45.2 (0.45) earlier, cut | at load 25+ | the narrow-signal step's one iteration: 17.6 s of work with 3.9 s left; the rest is the load (search at the machine's rate) | 57 / 27.8 (0.38) | no |
| ChengminLi | 74 / 28.7 (0.44); 95 / 25.0 (0.39) earlier, cut | at load 28+ | 15 lattice builds of 16 classes (40 thread s), stamping 14 maps, the load | 63 / 29.2 (0.60) | no |
| OpenDeck | 35 / 32.3 (0.79) | under load only | the load; resolve 23 s for 16 s of search | 47 / 33.6 (0.84) | no |
| brickpico | 67 / 32.1 (0.44) | under load only | the via reduction's pour checks (8.5 s of pour work), the load | 53 / 32.4 (0.75) | no |
| Wonkle | 41 / 34.0 (0.80) | under load only | its one attempt's resolve past its deadline (34 s of work for a bound of 32), the load | 51 / 34.2 (0.78) | no |

What was wrong, by cause:

- *The manual spokes* (Vortac, FoCTow). `add_manual_spokes` (straight
  tracks from starved pads into their pour, after every finish) runs a
  full pour analysis of GND per pad and one per candidate track: on the
  panels (642 x 647 nodes, four layers, 122 GND pads) 27 s of analyses
  in 28 s of wall clock for 34 tracks, in every attempt, beside 5 s of
  search. No budget counted it, no work deadline ended it, its fill
  probes were not even counted as pour work, and the rungs that ran on
  past the deadline did it too (x1366's log: 48 s in the continuation,
  58, 63 and 69 s in the three rungs side by side). The pour model was
  right (its modelled work 30.0 s against 27.2 s of wall): the work was
  real and uncharged.
- *A step past the bound* (megaduck). The ladder's first attempt took
  27.6 s; the narrow-signal step started with the 3.9 s kept for it,
  and a fresh router's first negotiation iteration, which no deadline
  stops, did 17.6 s of work (33 searches of half a second): 45 s of
  work against a bound of 32 s.
- *The pinned rate* (everything cut only under load). The tiers pin
  the reference's rate; the machine measures 0.4 to 0.6 of it at load
  25 and 0.3 to 0.4 at 60. A 90 s guard over a 30 s budget then holds
  only up to 0.36 of the reference, so every board that used its budget
  was cut whenever the tier ran beside other work. In the rows: the
  cut rows' `work_wall_ratio` was 0.27 to 0.45 where the measured share
  was 0.39 to 0.45 (megaduck 0.45 at 0.45, ChengminLi 0.28 at 0.39,
  brickpico 0.33 at 0.39). The guards did not follow the measured rate:
  `work_wall_ratio` was recorded, nothing acted on it.
- *Wall clock beyond the work, everywhere* (the rest of the ratio):
  the negotiation's pour refresh (counted as pour work, but neither in
  the negotiation's limit nor in the ladder's tally: Vortac's probes
  33 s of pour work beside 18 s of search, brickpico's via reduction
  8.5 s), the lattices (ChengminLi's 16 classes: 15 builds, 22 thread
  seconds), stamping. Measured, not charged here (below).
- *KiCad's check model* (layouts only; route mode with
  `skip_native_verification` runs no check). The constant 3.6 s fits
  ordinary boards (twelve measured at 2.8 to 5.4 s) but missed the
  complex outlines by four to nine times; the 0xCB panel's checks took
  275 s in v18b against 69 s charged.
- *The search itself* ran at the machine's measured rate on every board
  (Vortac's phases 2.99e6 a second at a measured share of 0.64, ChengminLi 0.78-1.09e6 at
  0.39, megaduck 1.2-1.7e6 at 0.45): no memory-bound lattice ran below
  the calibration; lowering is under 0.2 s on all of them.

The fixes:

1. **Manual spokes reuse their analyses** (exact): an analysis is a
   function of the routes, so after a kept track the next pass takes the
   trial's analysis, after a track not kept the one before, and the
   inner spokes start from the manual spokes' last. Vortac's phase 28 s
   to 13 s; on CompassTag, RS485M, Zulassungskarte, ch32v003, backplane
   and nervous-ecg (`PCB_ROUTER_NO_WALL_GUARD`, fingerprints on) every
   board and fingerprint is x1366's. The phase is a profile phase of its
   own (`manual spokes`), its fill probes count as pour work, it has a
   budget in work (the negotiation's, at least 60 s:
   `MANUAL_SPOKES_SECONDS`, a bound no ordinary board reaches; not the
   clean-up's cap, which a layout's polish shrinks) and stops at the
   wall-clock deadline, not at the work deadline.
2. **Route mode's ladder counts the manual spokes** beside the search
   (`result_work`; the router's `RoutingResult::manual_spokes_work`,
   board-router.json's `manual_spokes_work_seconds`). Vortac: the
   continuation's 6 s of search and 13 s of spokes reach the bound, no
   rung runs after it (they never won).
3. **No step without room for its first iteration**: route mode's
   narrow-signal and minimum-clearance steps start only if what is left
   under the bound covers the ladder's first attempt's work up to the
   end of its first negotiation iteration (`first_iteration_fits`,
   `RoutingResult::first_iteration_expansions`); else they are skipped
   with why. megaduck: 45.2 s of work to 27.9, the same board (the step
   lost anyway).
4. **The wall-clock guards stretch for pinned rates**
   (`KiCadWorkRate::wall_stretch`): by the pinned rate over the one
   measured at the start, at least 1, at most 4 (`MAX_WALL_STRETCH`,
   `PCB_WALL_STRETCH_MAX`), the search's in route mode, the larger of
   the search's and the placer's in a layout: route mode's
   `deadline_seconds`, the layout's `wall_guard`, the router's `GUARD`.
   With the rates measured nothing changes. The verdict and the rows
   have `wall_stretch`; run.py's timeouts allow for the cap when it
   pins the rates (`WALL_STRETCH_MAX`: 420 s for the intake's 30 s
   budget, 12 300 s for a layout of 1500 s; hang guards).
5. **KiCad's check model: complex outlines.** 20 s more per check
   (`COMPLEX_OUTLINE_SECONDS`) for a board with more than 200 graphic
   items on Edge.Cuts (`COMPLEX_OUTLINE_ITEMS`), measured on 20 of
   v18b's routed boards checked again uncached one at a time (load 22):
   katia (404 items) 15.5 s, ULK (411) 33.1, ghoul (496) 28.8,
   slab-pcb-left (1030) 15.1, interchange (1162) 33.8, SmartSpin2k's
   panel (1367) 28.8, the 0xCB panel (6369, and 8100 hatch holes) 41.7
   against its 17.4: model over measured 0.64 to 1.43 (was 0.11 to
   0.42); the twelve boards of at most 100 items (2.8 to 5.4 s) and
   every short-tier board (at most 140) unchanged. Linear terms in the
   items or in polygon vertices fit worse (beyond about 400 items the
   time stops growing; the vertex count also marks fast boards: urchin,
   0xcb-static, Blue_Line's 359 k). Not explained: d20_pcb (194 k
   vertices, 17.4 s), Quanta75 (14.4 s), PixelWave (12.1 s).
6. **Where the wall clock went, in every run**: the `wall accounting:`
   line and board-router.json's `wall_accounting` (the ladder's wall and
   work, the machine's share, the stretch, thread seconds of lattices,
   lowerings, preparation, each router phase with its search and pour
   work, kicad-cli's runs); the verbose `manual spokes:` line gives its
   wall clock and pour work.

**Measured.** The named boards: the table above; none cut by x1371.
x1366's wall over x1371's is not the comparison (the load moved
between runs); the ratio of work to wall clock against the machine's
share is: Vortac 0.21 at a share of 0.77 before (the uncounted spokes),
0.36 at 0.38 after (the work is the wall clock's at the machine's
rate); megaduck 0.51 to 0.52 at the same share, 46.2 to 27.8 s of work.
megaduck's row also changed with main's lowering gaps 2 (x1367: 72
unconnected, 16 vias, the same as x1371).

**Expansion tier** (16 x 1, `--no-lock`; x1365's at 8 x 1, load 37 to
69; x1370 and x1371 at load 14 to 27): x1370 and x1371 give the same 46
rows. Against x1365, with `PCB_LADDER_UNCHARGED_SPOKES=1
PCB_LADDER_NO_FIRST_ITERATION_GATE=1` (build/expansion-cuts-x1370-old):
45 rows identical, brickpico (cut in x1365) uncut at 32.1 s of work; so
the manual spokes' reuse, their budget, the stretch and the accounting
change no row. With the defaults: routed, vias, length, KiCad's findings
and verdict identical on 44 rows, `work_seconds` +0.1 to +1.3 s on 30
(their manual spokes, now counted); no row cut (x1365: brickpico). Where
the count changed where the ladder stops:
- FUEL4EP: the skeleton rung (111 open, 2.3 s) no longer starts (the
  bound is reached with the spokes counted); the same board, 30.8 to
  29.9 s of work.
- Julkort: the 0.075 mm rungs' shares are smaller by the regular
  pitch's spokes; the kept connect attempt negotiated 6 iterations, not
  4: 23/24 both, vias 20 to 23, length 257.2 to 255.1 mm, KiCad the
  same.
- RS485M: the 0.05 mm rungs' shares are smaller; their seeds and the
  tracks rung end at 3 open (were 2), the narrow-signal step varies
  another best attempt and loses (3 open; was 1): 19/20 to 18/20,
  KiCad 1 unconnected and 2 starved thermals to 2 unconnected and no
  error.
The tier's work 1264 to 1272 s, KiCad unconnected 723 to 724, new
errors 4 to 2, passes 16 to 16. Wall 2077 to 1519 s summed over the
rows, the longest row 111 to 72 s (the load differed).

**Short tier** (6 x 2, `--no-lock`) x1370 and x1371 against
build/short-x1365: all 12 rows identical (routed, vias, length, work,
KiCad's findings, verdict), none cut; no short-tier board has more than
140 Edge.Cuts items (1337-v4.0), so the check model is unchanged there.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode from v16,
2 x 4, budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_NO_WALL_GUARD=1`) x1370 against build/fp-lower2-x1366 and
x1371 against -x1367 (main's, jetson's rings): every fingerprint and
kept board identical;
`work_seconds` +0.2 to +2.8 s (the spokes counted; k30 2.8, Castor
1.6), and their wall clock fell (k30 270 to 181 s, Castor 187 to 115 s
at a lower load: the reuse).

**The layout sweep's cut rows** (v18b, 10 x 2, load 25 to 69): katia
(work over wall 0.35; measured 0.41 of the reference's search rate and
0.20 of the placer's), Batch_1 (0.32; 0.41 and 0.24), Quanta75 (0.39;
0.74 and 0.34), the 0xCB panel (0.42; 0.65 and 0.40). Their placement
ran at the placer's share (katia: 548 s of wall for 123 s of work) and
their routing near the search's: the pinned rate, not a phase. The
stretch (the larger share's inverse, at most 4: 4.0, 4.0, 3.0, 2.5)
puts their guards at 12 000, 12 000, 8 800 and 7 500 s, past their
3000-3200 s. The 0xCB panel's checks also took 275 s for 69 s charged;
with the outline term (6369 Edge.Cuts items) each charges 37.4 s. Not run again
here (a full layout each).

**What is still not counted** (measured, left for a decision): the
negotiation's pour refresh and the finishing phases' pour checks in the
ladder's tally (Vortac's probes 34.6 s of pour work beside 18.3 s of
search; brickpico 12.8 s beside 31; on the expansion tier 0 to 12.8 s a
board, median about 1.5): counting them is honest but moves most pour
boards' rows (every rung share after the first); the lattice builds
(ChengminLi 40 thread seconds for 15 builds); the stamping (the
negotiation's limit is search expansions). With the stretch these cost
wall clock, not cuts.

**The guard, recommended.** Keep `wall_guard` 2 and route mode's 3 x
budget as margins over the work at the machine's measured rate, and let
them stretch with a pinned rate (done here, cap 4): with the rates
measured a second of work is a second of this machine, and the guard is
only the margin for what the work does not count; with the rates pinned
the guard was a load-dependent budget, which is what cut the loaded rows
(their `work_wall_ratio` sat at the machine's share). run.py's
timeouts scale with the cap, not with a measurement of their own (the
binary measures; the harness only needs a hang guard above the
binary's guard). A tier on a loaded machine now takes longer instead of
cutting: up to four times a quiet run's wall clock.

Unit tests: ladder_tests (route mode counts the manual spokes; a step
needs room for a first iteration), work_bound_tests
(`first_iteration_fits`), work_rate (`wall_stretch`), check_work (the
outline term, footprint graphics on Edge.Cuts), test_run.py (the
stretched timeouts). `bash scripts/check.sh` and the clippy gate pass.
Docs: router.md (the work bound in route mode, the manual spokes, where
the wall clock went, the guards' stretch, KiCad's checks as work),
agent-tasks/README.md, config-reference (three variables).

Logs: build/expansion-cuts-x1370, -x1370-old, -x1371;
build/short-cuts-x1370, -x1371; build/fp-cuts-x1370, -x1370-old,
-x1371.

## Committed on 2026-10-22 (lowering gaps 2)

The finding types the previous lowering round left ("Lowering gaps in the
intake's findings" below): solder mask bridges, dimensions and texts on
copper, footprint graphics of a net and SeanBP's numbered nets. Each
checked with kicad-cli 10.0.7 on a board made for it; the changes are in
the lowering (`board_router/lower.rs`, `geometry.rs`) and `outline.rs`.
docs/router.md, "Dimensions, text boxes, cached copper texts and
padstacks" and "Solder mask bridges".

Binaries: x1345 is main before (3223282, the connectivity agreement);
x1350 and x1351 steps (x1351 without plated holes of no listed layer),
x1352 the commit's code.

**The measurement.** From the intake's KiCad reports (the 2026-10-18
quick run, an older binary): the boards whose result has more findings of
a type than the designer's board, the router-output types left out
(track_dangling, track_not_centered_on_via, unconnected_items,
starved_thermal): 184 boards. The intake's own count over them (its
binary, before the project rules and the first lowering round):
track_width 7120 on 89 boards, clearance 1362 on 41, hole_to_hole 123 on
8, copper_edge_clearance 100 on 17, solder_mask_bridge 78 on 15,
drill_out_of_range 51 on 1, shorting_items 32 on 13, length_out_of_range
30 on 4, hole_clearance 29 on 9, annular_width 22 on 1,
malformed_courtyard 13 on 4, creepage 3 on 3, skew_out_of_range 1. With them the intake boards that have a
dimension on copper (40), a padstack (16) or a cached text on copper (22):
252 boards, routed again from the intake's stripped boards and settings
(route mode, 30 s of work, one thread, `PCB_WORK_RATE=3.15e6,150e6`),
KiCad's DRC with zones refilled, against the intake's DRC of the
designer's board (scratchpad, not a tier). Excess findings by type (sum
over boards of ours less the designer's, where more), x1345 against
x1352:

| type | x1345 findings / boards | x1352 findings / boards |
| --- | --- | --- |
| solder_mask_bridge | 89 / 18 | 56 / 8 |
| shorting_items | 36 / 11 | 0 / 0 |
| clearance | 32 / 10 | 2 / 2 |
| length_out_of_range | 30 / 4 | 30 / 4 |
| track_width | 24 / 1 | 24 / 1 |
| hole_to_hole | 7 / 1 | 7 / 1 |
| hole_clearance | 6 / 4 | 3 / 1 |
| creepage | 3 / 3 | 3 / 3 |
| skew_out_of_range | 1 / 1 | 1 / 1 |

KiCad's unconnected items over the 252: 980 to 973. 223 rows identical,
29 changed, each with an item of these gaps but VortacDesign's panel (cut
by the wall clock's deadline in both runs, as FoCTow's) and megaduck
(cut at x1352 only: the machine's load was 60 to 70 during that run): padstacks (dennisss's seven
camera boards: excess 53 to 0, unconnected 133 to 125; adrianmarple 10 to
0), dimensions on copper (shrimp-18 3 to 0, robotique-udes 2 to 0,
liquid-rocketry 18 to 0 with a cached text, and eight boards whose
dimensions moved routes only: megaduck, PCBench BalthazarPSU3, Soraxem,
kapibaratomo, koron, sbeddes, ub1qbm), cached copper texts (TomKeddie 3 to
0, FoCTow 1 to 0, Pips801, eutectic6337, mothenjoyer69 routes only), the
bare-side aperture (c-lydia 2 to 0), a whole-circle arc (cnlohr 3 to
0) and plated holes listing no copper layer (aaronds's two ESP32 DevKitC
boards, clearance 6 and 2 to 0; shoonie's two c10 keyboards,
solder_mask_bridge 3 fewer each). Four rows have more unconnected items (dennisss ar0234_r1 2 to 5,
mira220 29 to 30, megaduck 64 to 65, PCBench 163 to 167): more copper to
route around in 30 s. FoCTow's route needs 150 to 180 s of wall clock
under load (the scratch harness's timeout was raised from 200 to 400 s
for it).

**What changed.**

- **Dimensions on copper** are copper of no net (`dimension_shapes`):
  KiCad 10.0.7 measures from the lines' centres (zero width: tracks at
  0.19, 0.15, 0.10, 0.064 and 0.05 mm from the crossbar, an extension
  line's start, an orthogonal second extension and an arrow, exactly as
  computed) and from the text's glyphs. Aligned and orthogonal geometry
  as PCB_DIM_ALIGNED and PCB_DIM_ORTHOGONAL build it; KiCad 5's stored
  lines read as they are; leader, centre and radial a line between their
  points.
- **Text boxes** (`gr_text_box`, `fp_text_box`) on copper: the box filled,
  grown by half its stroke (KiCad measures from that outline, border or
  not); on a mask layer an opening. None in the intake is on copper or
  mask; checked on a made board.
- **Cached (TrueType) copper texts**: KiCad draws them from the installed
  font (TomKeddie's sign: plotted glyphs up to 0.92 mm outside the cache);
  each cached line is also a box grown by a tenth of the text height
  (`cached_text_line_boxes`), the gaps between lines open.
- **Padstacks** (KiCad 9's front_inner_back and custom): the inner and
  back shapes are copper of the pad's net on their layers
  (`padstack_variants`); the back's mask aperture follows the back shape.
- **A pad's mask opening on a side without its copper** (c-lydia's U3:
  `(layers "F.Mask" "B.Cu" "F.Paste")`) is an aperture of its net there.
- **A plated hole that lists no copper layer** (`(layers "*.Mask")`) is
  copper on every layer: kicad-cli shorted tracks on F.Cu, an inner layer
  and B.Cu to such a pad of no net; a pad listing some copper layers has
  copper only there. It was a mask-only opening (aaronds's DevKitC pins:
  tracks 0.005 mm from them).
- **An arc ending where it starts** is a whole circle (`arc_points`; MK1's
  ring pad in cnlohr's tadpoleswadge, a custom pad primitive; jetson's
  two In4.Cu rings).

**Decided, not changed.**

- **Footprint graphics of a net** (item 4 of the list): KiCad 10.0.7
  treats them as copper of no net. kicad-cli on an antenna footprint whose
  polygon is on net A, KiCad 9 `(net 1)` and KiCad 10 `(net "A")` alike:
  clearance findings at 0 mm against its own pad and an A track, the pad
  unconnected although the track touches the polygon. The lowering
  already kept them as copper of no net at Default's clearance; a comment
  says why. The intake's four boards with such graphics (the TI antennas
  of two student boards, keytartouch's 24 touch pads, a minibadge
  holder) have these findings on the designer's board too.
- **SeanBP's numbered nets** (item 5): the connectivity agreement (main,
  x1345) made digit names routable and reads `(net N)` as a code; the same
  fix was here and was dropped in the rebase. SeanBP's Edge-Card-CAEN
  ("the board has no routable connections" in the intake): x1345 routes
  69/70, KiCad 1 unconnected, no error. Its 70 nets each join a test
  point on F.Cu and one on B.Cu at one place, both "REF**.1": the
  pads-of-one-number merge took them for one terminal until x1345's
  shared-layer condition.
- **Solder mask bridges over pours** (item 1): openings the designer drew
  along their own tracks (AntonSievering's F.Mask strips along RF lines
  over the GND pour, bostang's strips from U1's pins, eigenhari's B.Mask
  segments) show the pour once the tracks are stripped, and a pad inside
  bridges whatever is routed there. Treating such an opening as no net's
  where it reaches farther than the pour's clearance from its pads (tried
  as x1350's predecessor on 19 boards) took AntonSievering 7 to 11 and
  eigenhari's PQ_PL2 6 to 9 (56/59 routed, was 59/59) and fixed two
  single findings: not kept. These, and the openings over pads of
  several nets (Sisu's J_LCD1, ringshout's J2: a track leaving a pad
  inside bridges), are the designer's board: the strict rule subtracts
  the stripped designer board's own pairs (opening and pad, opening and
  zone) but not the opening and our track. A strict rule that subtracts
  every bridge of an opening that already bridges in the baseline would
  remove them (Sisu's 15 strict solder_mask_bridge in the fingerprint
  rows are of this kind); a harness change, not made.

**What remains** of the 252 at x1352: solder_mask_bridge 56 on 8 boards
(the openings above: bostang 33, AntonSievering 7, eigenhari's PQ_PL2 7
and ringshout 1; shoonie's c8 5 and c8v3 1, B.Mask graphics and NPTH
pads beside the switch pads; McbeEringi's two boards 1
each, a B.Mask text and an F.Mask polygon over the pour),
length_out_of_range and skew_out_of_range (length-tuning rules, 5
boards), track_width 24 on one board, hole_to_hole 7 on one, creepage 3
on three (pad pairs the router cannot change), clearance 2 (KiCad's QA
board issue11814 in two copies: 0.1875 against its PadToTrack rule),
hole_clearance 3 on jrsteensen's OpenHornet (a custom rule's 2.65 mm
from an NPTH hole "for GND copper"). Lathe-Mariel's In4.Cu short against
a PTH pad (intake) is not reproduced at x1345.

**Rules boards** (rules-tasks.json, 8 x 1, `--no-lock`), x1345 against
x1351 and x1352: all 20 rows identical (passes 18, strict 18, unconnected
4, vias 494). ChengminLi's flyback is CUT by the deadline in all three
runs (the machine's load at 45 to 60 while the sweep and these runs
shared it; 35/38 each time).

**Expansion tier** (8 x 1 for the reference, 10 x 1 and 8 x 1 here,
`--no-lock`), build/expansion-x1345 against x1351: all 46 rows identical,
no row cut; against x1352: 43 identical, three cut at a load of 60 to 70
(OpenDeck, brickpico, Wonkle) and run again alone: identical.

**Short tier** (4 x 2 for the reference, 5 x 2 and 6 x 2 here,
`--no-lock`), build/short-x1345 against x1351 and x1352: all 12 rows
identical, no row cut.

**Fingerprints** (Castor, k30-SBC, jetson, Sisu; route mode, 2 x 4,
budget scale 0.25, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`),
build/fp-conn-x1345 against build/fp-lower2-x1351 and -x1352 (the same
fingerprints): Castor, k30 and Sisu every fingerprint and board
identical; jetson's kept board (md5) and row identical, the fingerprints
of one attempt's clean-up changed: its two whole-circle arcs on In4.Cu
(radius 0.1 mm, `(start)` equal to `(end)`) are rings now, not points.
Castor's two dimensions are on Dwgs.User; none of the four has a
padstack, a cached copper text or a plated hole without copper layers.

Unit tests: board_router lowering_gap_tests (dimensions and text boxes
against kicad-cli's distances, a padstack's back copper and a bare-side
aperture, cached copper text lines and a footprint graphic of a net),
outline (a whole-circle arc), the plated hole without copper layers. `bash scripts/check.sh` and the clippy gate
pass.

**Rebased on 642e28c** (the pour model, x1366) as x1367: rules boards
x1366 against x1367 identical on all 20 rows (ChengminLi's flyback the
same row, cut by the deadline at x1367 only); against x1352 nuclone (48/48
to 18/18 connections) and issue25101 (length) moved with the pour
package, as x1366 shows. Short tier build/short-x1365 against x1367: all
12 rows identical, none cut. Fingerprints x1366 against x1367: Castor,
k30 and Sisu identical; jetson's board identical, the one clean-up's
fingerprints changed by its rings as above.

Logs: build/rules-lower2-x1345, -x1351, -x1352, -x1366, -x1367;
build/short-lower2-x1367; build/fp-lower2-x1366, -x1367;
build/expansion-lower2-x1351, -x1352, -x1352-recut; build/short-lower2-x1351,
-x1352; build/fp-lower2-x1351, -x1352.

## Committed on 2026-10-22 (pour model after connectivity)

The connectivity agreement (below) made the router's "complete" what
KiCad finds connected on 63 of the 65 boards, but cost rows: MSign (15
to 60 unconnected), stellarcorp's magnetorquer (a pass to 84/86),
LC_PAMV4b (a starved thermal), urchin (42 to 65 vias) and
eternal-keypad in the short tier, and left issue6284 (a spoke joining
U102.4 and U102.6 that KiCad keeps apart), the older starved thermals
of ZSWatch and anthracite and KiCad 9's footprint jumpers. Branch
pour-regressions; snapshots x1360 (the transitive terminals), x1361
(with the spoke end test and jumpers), x1362 (with the vias' reliefs),
x1363 and x1365 (the commit, x1362 without its switches; every row of
the 65, the expansion and the short tier and the fingerprints repeat
x1362's); x1364 an experiment not kept (below). Attribution with a
binary whose changes could be switched off one at a time (x1361, x1362:
`PCB_X_OLD_END`, `PCB_X_NO_VIA_RELIEFS`, `PCB_X_NO_JUMPERS`, not
committed) and the per-net terminal counts of board-router.json.

**What was wrong, and the fixes.**

- *Pads of one number joined through a third were two terminals.* The
  connectivity commit merged pads of one number only into the largest
  that overlaps them on a shared layer. An exposed pad on F.Cu and its
  copy on B.Cu share no layer, but both overlap the thermal vias between
  them: one item to KiCad (kicad-cli on a board with a track from the
  F.Cu pad: no unconnected item; without the vias the B.Cu pad is
  unconnected). The lowering made the B.Cu pad a terminal of its own and
  routed it: MSign's U1.21, LC_PAMV4b's U3.25, Sisu's U2.11, ZSWatch's
  IC401.33, IotThermalCamera's TU2.9; urchin's and eternal-keypad's
  reversible diodes (D_SMD_TH: SMD pads on both sides and a through-hole
  pad that both overlap), mounting holes with copper on
  both sides (magnetorquer's MH1 to MH4, VTC-WISDOM's H2). Pads of one
  number are now grouped by overlap (union-find over the pairs that
  share a layer and hold each other's anchor), the largest of a group
  its terminal; stacked pads with nothing between them (Knox Key's
  P1.A1, VTC-WISDOM's edge-mount SMA J3) stay two terminals, which
  KiCad wants joined by a via. The urchin and MSign costs were these
  terminals, not a choice of side: no pad pair without a hole needs the
  router to pick one.
- *Footprint jumpers.* KiCad 9's `(duplicate_pad_numbers_are_jumpers
  yes)` joins the pads of one number of a footprint,
  `(jumper_pad_groups ("1" "2") ...)` the pads of a group's numbers,
  without copper (kicad-cli 10.0.7: a track from one pad 1 leaves the
  other pad 1 and pad 2 unconnected without either, pad 2 only with the
  first, nothing with the groups). Such pads of one net are one terminal
  now (37 boards of the intake, mostly KiCad QA copies; ducktop2's bms:
  its WAGO terminals, PACK_NEG_RAW and three more nets one terminal
  fewer; Sisu's I-PEX J2).
- *A spoke whose end lies in another item's clearance.* KiCad keeps a
  spoke where its end point lies in the fill (zone_filler.cpp tests
  `spoke.CPoint(3)` against the fill less the clearance holes, opened by
  half the minimum width); the model asked for a brush centre within
  half the minimum width of the end that keeps the clearances less
  `EPSILON` (3 microns). anthracite's R503.2 and R703.2: diagonal spokes
  (`thermal_bridge_angle 45`) whose ends lie 0.3981 mm from R503.1's
  rounded corner under a 0.4 mm clearance; a brush centre beside the end
  fit by 0.3 microns of the tolerance, KiCad drops the spoke and finds
  the pad starved. The end itself must now keep every clearance
  (`spoke_end_lands`, `END_TOLERANCE` 0; pcbnew 10.0.7 on a test board:
  an end 0.202 mm from another net's pad under a 0.2 mm clearance keeps
  its spoke, 0.198 mm loses it).
- *Vias of the net in a hatched zone.* KiCad 10 gives every via of the
  zone's net a thermal relief in a hatched zone whose pads connect by
  thermals (knockoutThermalReliefs: the via's radius and the thermal gap
  knocked out, a ring a spoke wide, four spokes; with `connect_pads yes`
  or `thru_hole_only` the via is solid). The model filled up to the via.
  issue6284's U102.6: its +x spoke ends 0.67 mm from a GND via (relief
  0.8 mm); KiCad drops the spoke, the pad's other piece holds nothing
  else, and KiCad finds two zone pieces unconnected where the model
  joined the pad to the via's piece and called the board complete (the
  "spoke join" of the connectivity commit). `Hatch::via_reliefs` (the
  lowering: a zone's `connect_pads` with no keyword), the fill probe
  keeps brush centres out of the reliefs of the net's vias, routed or
  already on the board (`FillProbe::relieved_vias`, `Item::ViaRelief`),
  their rings count as fill (`in_ring`), and the pour map clears the
  relief's nodes but for the via's four spokes along the axes
  (`cut_via_reliefs`): IotThermalCamera's Tgnd, routed again after the
  transitive terminals, joined two pieces through a diagonal neck of
  the lattice past a via's relief and was complete by our count with
  KiCad 1 unconnected. pcbnew on a test board: a pad's spoke end in a
  GND via's relief loses the spoke, with `connect_pads yes` it keeps it.

Not kept (x1364): the starved-thermal model by each pad's own zone
connection (`solid_pad_obstacles`, `isolated_pad_obstacles`) rather than
its label's. It gave eternal-keypad's ProMicro1.4 and .23 and 74th's
J3.S1 model rows (by label their SMD pads set to no connection made the
through-hole pads isolated), but those rows are still missed (the model
counts two spokes where KiCad counts one), and ray4477's U1.33 thermal
vias inside a solid exposed pad went from predicted to missed (by label
the B.Cu exposed pad was thermal, which happened to give the vias
KiCad's one spoke). Census below: predicted 165 to 163, false alarms 29
to 31, no model row 4 to 0.

Unit tests, one per cause, with the kicad-cli or pcbnew checks in their
comments: pcb-kicad's
`pads_of_one_number_joined_through_a_third_are_one_terminal`,
`pads_of_a_jumper_group_are_one_terminal`; pcb-router's
`a_spoke_end_inside_a_clearance_is_dropped`,
`a_via_of_the_net_has_a_thermal_relief_in_a_hatched_zone` (the spoke
and the pour map's nodes).

**Starved-thermal census** (`experiments/pour-fidelity/census.py`, the
model on the final copper against KiCad's refilled DRC; the 32 boards of
the 44-board census still on disk and the 42 of the intake's
route-t8, 218 starved pad rows): predicted 162 to 165, missed 52 to 49,
false alarms 29 both, no model row 4 both (x1360's model, which has the
transitive terminals only, against x1363). The three: ptSolar's PV2.2 on
three layers (spoke ends inside a clearance). The hatched boards' rows
(0xCB, Qfwfq) change pieces, not verdicts.

**Route mode on the 65** (connectivity-tasks.json, 30 s, 8 x 1,
`--no-lock`; `census.py --run`), x1345 against x1365: 57 rows
identical; verdict agrees with KiCad 63 to 64; complete by our count
with KiCad unconnected 2 to 1 (robotique's BioGenius, the dimension on
copper of the lowering gaps 2 package); KiCad's unconnected items 12 to
11; passes 56 to 57 (strict 53 to 54); vias 1811 to 1900. The changed
rows:

| board | x1345 | x1365 | cause |
| --- | --- | --- | --- |
| baxter-barlow issue6284 | 127/127, KiCad 2 (complete by our count) | 126/127, KiCad 2 (U102.6 open: agrees) | via reliefs |
| IotThermalCamera | 88/88, KiCad 0, hole_to_hole 1 | 88/88, pass | transitive terminals (TU2.9, two nets one fewer) and via reliefs (with the terminals alone 88/88 and KiCad 1 unconnected: TU8.9 and TU8.14 apart) |
| BleJoystick | 27/27, 37 vias, pass | 27/27, 40 vias, pass | via reliefs |
| Cuprum77 BananaBoard Passive | 3/3, 48.9 mm, pass | 3/3, 42.0 mm, pass | transitive terminals |
| OpenAir rf-amplifier | 32/32, 250.5 mm, pass | 32/32, 251.8 mm, pass | transitive terminals |
| smkent dip-adapters | 17/17, 3 vias, pass | 17/17, 5 vias, pass | transitive terminals |
| 7-seg-clock | 66/66, 2208.5 mm, pass | 66/66, 2158.8 mm, pass | transitive terminals |
| mikesmitty controller | 73/78, KiCad 8 | 74/78, KiCad 7 | transitive terminals |

(with the end test and the vias' reliefs switched off, x1362, every row
but issue6284, IotThermalCamera and BleJoystick is x1365's).

**Expansion tier** (46 boards, 8 x 1, `--no-lock`), x1345 against
x1365 (x1361 and x1362 the same rows; x1361 cut brickpico by the wall
clock, the same row, x1365 cut none): 36 rows identical; passes 15 to
16 (strict 15 to 16), unconnected 764 to 723, new errors 7 to 4 (strict
3 to 2), vias 4029 to 4090.

| board | x1345 | x1365 | cause |
| --- | --- | --- | --- |
| mincrmatt12 MSign | 165/188, KiCad 60, 200 vias | 176/188, KiCad 17, 289 vias | transitive (U1.21; x1332 177/188, 15) |
| WMXZ LC_PAMV4b | 51/52, starved_thermal 1 | 51/52, pass, 26 vias (x1332's) | transitive (U3.25) |
| VTC-WISDOM node | 56/77, KiCad 33, 98 vias | 56/77, KiCad 35, 69 vias (x1332's) | transitive (H2 mounting holes, GND 113 to 108 terminals; J3's SMA pads stay two) |
| ZSWatch | 107/107, pass, 134 vias | 107/107, pass, 142 vias | transitive (IC401.33) |
| TokayEdge | 84/84, pass, 87 vias | 84/84, pass, 82 vias (x1332's) | transitive |
| ducktop2 bms | 99/99, pass, 234 vias | 99/99, pass, 240 vias, 3141.6 mm | jumpers (the WAGO terminals) and transitive (U2218's thermal vias; alone 239 vias, x1332's row) |
| brown-studios pdlc | 21/33, KiCad 12, 34 vias | 21/33, KiCad 12, 29 vias | transitive |
| Wonkle | 135/271, 496 vias | 135/271, 497 vias | transitive |
| stellarcorp magnetorquer | 84/86, KiCad 2, 139 vias | 84/86, KiCad 2, 137 vias | transitive (MH1 to MH4, GND 93 to 89, x1332's terminals) |
| trope-oshw anthracite | starved_thermal 4 | starved_thermal 2 | the spoke end test (R503.2, R703.2 now predicted and repaired) |

The magnetorquer stays at 84/86 (a pass at x1332): its terminals are
x1332's again, but the ladder keeps the plane-stubs rung, whose probe
leaves 41 nets unfinished against the tracks probe's 42 (x1332: 44
against 42); the other pour changes of the connectivity commit made the
connect probes better, and the connect rung leaves I2C3.SDA and
Net-(U8-LX1) open, which KiCad confirms (2 unconnected): honest, a
ladder choice by probe counts, not a model error.

**Short tier** (tasks v7, 4 x 2, `--no-lock`), x1345 against x1365
(x1362 at 6 x 2 the same rows): 8 rows identical; passes 7 to 8
(strict 6 to 7).

| board | x1345 | x1365 | cause |
| --- | --- | --- | --- |
| 0xCB-1337 | 47/47, 50 vias, strict starved_thermal 1 | 47/47, 66 vias, strict pass | via reliefs (hatched GND; off, x1345's row) |
| USB_Keypad | 37/39, 113 vias | 37/39, 93 vias | via reliefs (hatched; off, x1345's row) |
| eternal-keypad | 69/69, 66 vias, shorting_items 1 | 65/65, 45 vias, pass (strict: starved_thermal 2) | transitive (its D_SMD_TH diodes: 4 connections fewer) |
| urchin | 68/68, 65 vias | 68/68, 42 vias (x1332's row) | transitive (its D_SMD_TH diodes) |

eternal-keypad's two strict starved thermals are ProMicro1.4 and .23 on
B.Cu (through-hole pads of ProMicro_facedown, whose SMD pads of the same
numbers are set to no connection): the model, by label, takes them for
isolated and has no row (the per-pad experiment above gave rows that
still count two spokes against KiCad's one).

**Fingerprints** (route mode from the v16 layouts, quarter budget, 2 x
4, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
build/fp-pour3-x1365 against build/fp-conn-x1345; build/fp-pour3-x1363
the same): Castor, k30-SBC and
jetson identical (every fingerprint and the board); Sisu changes from
the start: U2.11 is one terminal again and J2's jumper pads one each
(GND 265 to 263 terminals, Net-(J2-RF) 5 to 4): 182/192 to 188/192,
KiCad 13 to 6 unconnected, solder_mask_bridge 14 both (with the end
test off the same row and fingerprints).

**Gates**: `bash scripts/check.sh` and the clippy gate pass.

**Left**: anthracite's J106.20 (F.Cu: a spoke the zone's outline cuts
from the pad that still crosses its ring; the lattice joins the piece it
lands in to nothing, KiCad's outline holds more than a dozen items) and J107.1 (In2.Cu:
a sliver of fill around a via the model calls outside the zone),
RS485M's JP1.3 and R3.2 (one spoke the model counts as two, one it does
not see), ZSWatch's older one is gone with the transitive terminals;
eternal-keypad's ProMicro pads (per-label zone connections, above);
robotique's BioGenius (a dimension on copper, the lowering gaps 2
package's).

## Committed on 2026-10-22 (KiCad connectivity agreement)

In the expansion intake (be4b012, 5 834 boards) 192 boards were complete
by the router's count (every connection routed, no unconnected
terminal) while KiCad's DRC with zones refilled, which loaded every one
of them, reported unconnected items; on 119 of them that was the only
new finding (docs/benchmarks.md, "Expansion intake"). The router's
"complete" and `router_complete_unverified` were statements about a
model of KiCad's connectivity that was wrong in five ways. Branch
kicad-connectivity; snapshots x1340 to x1344 on ffa49a5, x1345 (the
commit, rebased on the lowering gaps, 837b5f6). On the rebased binary
every measurement below repeats row for row: the 65 (x1333 against
x1345: the x1322 and x1344 rows), the expansion tier against
build/expansion-x1332 (the same 24 changed rows; tuxkeychain is the
lowering gaps' 35/42 in both), the short tier against build/short-x1332
(x1344's rows) and the fingerprints against build/fp-lower-x1332
(x1344's).

**The census** (`experiments/kicad-connectivity/census.py`; nothing in the
boards is executed): the intake's rows (the last per board) with the
router complete and KiCad's unconnected items, each item classified from
the routed board (items by uuid, net codes resolved) and KiCad's report;
`--run <dir>` does the same for a route-mode run of run.py.
`experiments/kicad-connectivity/tasks.py` writes
`benchmarks/agent-tasks/connectivity-tasks.json`: the 65 of the 192 whose
repository has a licence run.py fetches (the other 127 are NOASSERTION).
By the cause of the first unconnected item (boards; items of that cause
on those boards):

| cause | boards (65 with a licence) | items |
| --- | ---: | ---: |
| pour, a zone among the items: the refilled fill does not reach what the pour model connected | 58 (20) | 105 |
| pour, two items of a pour net, no zone among them | 57 (19) | 191 |
| copper graphic with a net (a line, an arc, a circle, a filled rectangle) | 30 (8) | 99 |
| pads of one number stacked on two sides | 28 (10) | 570 |
| net named by a number | 13 (5) | 176 |
| track ending on a pad without copper on its layer | 2 (1) | 4 |
| other | 4 (2) | 70 |

**What was wrong, and the fixes**, in order of boards:

- *The pour model connected what KiCad's fill does not.* A solidly
  connected pad (`connect_pads yes`, `zone_connect 2`) joined every fill
  piece within 0.8 mm of its nodes (ORBIT's U2.14: a piece 0.3 mm past
  its edge, beyond the clearance of the pads beside it, joined U2's
  island of ground pads to the rest). Now a piece joins it only where its
  fill reaches the pad's copper: a free node within the brush's half
  width, or a straight run of fill (the fill probe, off the lattice) from
  a node at most two pitches beyond (`solid_fill_reaches`,
  `PourMap::pieces_near`). A pad set to no zone connection (`zone_connect
  0`) sat in the fill like any pad of the net (1999AZZAR's J2.RN: a
  branch of no length "connected" it); it is cut out at the zone's
  clearance and half the minimum width (`isolated_knockouts`), so only a
  track into the fill joins it. The pour masks held nodes up to the
  zone's outline and up to other zones' regions; KiCad's brush keeps half
  the minimum width inside the outline and the larger clearance from a
  higher zone of another net (as the fill probe already did): a column of
  nodes on the outline joined pieces KiCad keeps apart
  (mothenjoyer69's GND_B). And of two zones of other nets with one
  priority KiCad fills the one with the larger uuid first
  (ZONE::HigherPriority; the USB-C 3-way splitter's +5V and GND, both
  priority 1): the lowering cut neither (`Pour::uuid`). KiCad caps a
  spoke's width by the pad's size, a custom pad's by its anchor's: a
  custom pad whose anchor is narrower than the zone's minimum width gets
  no spoke (Cuprum77's paw pads, a 0.1 mm anchor in large primitives,
  checked on a small board with kicad-cli: unconnected with a 0.1 mm
  anchor, connected with 1 mm); the model drew them as wide as the
  primitives' box.
- *Pads of one number on two sides were one terminal.* The lowering
  merged pads of one number that overlap (an exposed pad's thermal vias)
  without asking for a shared layer, so a USB-C receptacle's or an edge
  connector's top and bottom pads at one place were one terminal: a
  track on B.Cu stood for both, KiCad wanted them joined (Knox Key's
  P1.A1). They are merged now only when they overlap on a layer both
  have (KiCad: a pad of one number apart, or on two sides without a
  hole, is an item of its own; `duplicate_pad_numbers_are_jumpers` and
  `jumper_pad_groups` do join pads, which the lowering does not model:
  the router routes them, more than KiCad needs).
- *Copper graphics with a net.* KiCad 8 and 9 name a graphic's net by
  code (`(gr_arc ... (net 2))`); the lowering read "2", took it for no
  net and the graphic for an obstacle of none; KiCad wanted it joined to
  Net-(D1-K) (justbuchanan's ledboard). Net codes are resolved through
  the board's net table (`net_codes`, `item_net`, also for old tracks and
  vias), a filled graphic with a net was a terminal already, and a
  stroke-only one is now too: at the middle of its first piece, unless it
  lies on a terminal's copper of its net (BleJoystick's arcs, the
  chessboards' rings).
- *Nets named by numbers.* `routable_net` left out every name of digits
  as "a placeholder for a single pin"; Soraxem's usb-dmx names nets "14"
  and "0", and the router called the board complete without them. A net
  form of one unquoted number is a code, every other number a name
  (`net_name_from_form`); board_stats counts such nets too.
- *The emitted copper is checked.* The finish drops stubs that come too
  close to other nets' copper after the statuses are taken, so a net
  could be "routed" with a pad left apart. `verify::copper_components`
  joins a net's copper (pads, merged pads, existing copper, graphics,
  the emitted tracks and vias) where it overlaps on a shared layer, as
  KiCad's connectivity without pours does (a custom pad through its
  anchor only); a net routed by tracks alone whose copper leaves a
  terminal apart is partial (`copper check:` under verbose or
  `PCB_ROUTER_DEBUG`). Switched off, the expansion tier's changed rows
  and jetson's fingerprints stay as they are: it caught nothing there;
  it is the guard that keeps the router's "complete" a statement about
  the copper it writes.

Unit tests, one per cause: pcb-kicad's
`stacked_pads_of_one_number_on_two_sides_are_two_terminals`,
`a_net_named_by_a_number_is_routed`, `a_net_form_names_a_net_unless_it_is_a_code`,
`an_item_names_its_net_by_code_in_an_older_board`,
`a_stroke_with_a_net_is_a_terminal`,
`of_two_zones_of_one_priority_the_larger_uuid_is_filled_first`;
pcb-router's `a_solid_pad_joins_only_the_fill_that_reaches_it`,
`a_pad_without_zone_connection_is_kept_out_of_the_fill`,
`the_fill_keeps_its_brush_inside_the_zone_outline`,
`the_copper_check_joins_what_overlaps_on_a_shared_layer`,
`a_custom_pad_spoke_is_no_wider_than_its_anchor`.

**Route mode on the 65** (connectivity-tasks.json, the intake's settings,
30 s, 8 x 1, `--no-lock`; `census.py --run`):

| | x1322 (main) | x1340 | x1341 | x1343 | x1344 |
| --- | ---: | ---: | ---: | ---: | ---: |
| our verdict agrees with KiCad | 15 | 32 | 52 | 62 | 63 |
| complete by our count, KiCad unconnected | 50 | 33 | 13 | 3 | 2 |
| KiCad's unconnected items | 346 | 77 | 29 | 15 | 12 |
| passes (designer-relative, strict) | 14 (14) | 28 (26) | 46 (45) | 55 (52) | 56 (53) |

x1340: stacked pads and numeric names; x1341: the solid reach, the
isolated pads' knockout, net codes and stroke graphics; x1343: the
brush in the outline and clear of other zones, the uuid order and the
copper check; x1344: the custom pads' spokes. By cause (first item,
boards that disagreed at x1322, then at x1344): stacked pads 13 (10, and
3 whose first item was a zone's two sides) to 0; pour 23 to 2 (12 fixed
by the solid reach and the knockout, 8 by the brush and the zones'
order, 1 by the custom pads' spokes); copper graphics 8 to 0;
numeric names 5 to 0; other 1 to 0. No board went the other way (our
count incomplete while KiCad finds it connected): the one board our
count calls incomplete, mikesmitty's controller (5 open), KiCad finds 8
unconnected, as at x1322. Per board (our count, KiCad's unconnected
items; (x) marks a disagreement):

| board | x1322 | x1344 | x1322 cause |
| --- | --- | --- | --- |
| 1999AZZAR_pcb-skill_project_pcb_tht_opamp_eq_tht_opamp_eq | complete, 2 (x) | complete, 0 | pour net |
| 74th_74th-oshw-projects_MISC-grove_expander_MIAC-grove_expander | complete, 0 | complete, 0 | - |
| Aightech_CleverHand-hardware_pcb_hmi_interface_DRY_ELEC_6CH_THIN | complete, 1 (x) | complete, 0 | pour zone |
| Apex-Ottawa-Team_hardware_Archive_RF_Module_RF_Module | complete, 1 (x) | complete, 0 | pour net |
| Apoll011_avi-hw-mesh-node_Hardware_ORBIT | complete, 1 (x) | complete, 0 | pour zone |
| atopile_packages_packages_rj45-connectors_layouts_usage_usage | complete, 18 (x) | complete, 0 | numeric net name |
| atopile_packages_packages_saleae-header_layouts_usage_usage | complete, 4 (x) | complete, 0 | numeric net name |
| azerimaker_olmBoard-RP605x_hardware_olmBoard-RP605x-v1_olmBoard- | complete, 2 (x) | complete, 0 | pour zone |
| AzonInc_Doorman_pcb_slot_doorman_slot | complete, 1 (x) | complete, 0 | pour net |
| baxter-barlow_kicad-assistant_qa_data_pcbnew_issue11814 | complete, 2 (x) | complete, 0 | same number stacked |
| baxter-barlow_kicad-assistant_qa_data_pcbnew_issue6284 | complete, 2 (x) | complete, 2 (x) | pour zone |
| BerkeleyHCI_PolymorphicBlocks_examples_BleJoystick_BleJoystick_b | complete, 11 (x) | complete, 0 | copper graphic |
| BerkeleyHCI_PolymorphicBlocks_examples_IotThermalCamera_IotTherm | complete, 1 (x) | complete, 0 | pour zone |
| bluepylons_USB-C-3-way-splitter_V5__autosave-USB-C-3-way-splitte | complete, 1 (x) | complete, 0 | pour zone |
| bluepylons_USB-C-3-way-splitter_V5_USB-C-3-way-splitter-V5 | complete, 1 (x) | complete, 0 | pour zone |
| Cuprum77_BananaBoard_Passive | complete, 3 (x) | complete, 0 | pour net |
| dennisss_dacha_pkg_media_camera_boards_camera_vb5943_r1_board-la | complete, 1 (x) | complete, 0 | pour net |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r10_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r1_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r2_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r3_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r6_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r7_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r8_board-latest | complete, 0 | complete, 0 | - |
| dennisss_dacha_pkg_vision_mocap_boards_compute_r9_board-latest | complete, 0 | complete, 0 | - |
| dragonlock2_kicadboards_projects_BSWelder_BSWelder | complete, 1 (x) | complete, 0 | pour zone |
| embeddedos-org_eCAD-Hardware-Products_boards_cad_m5stack_m5_cad_ | complete, 27 (x) | complete, 0 | same number stacked |
| enzocoralc_Tiny20_tiny20_choc_tiny20 | complete, 1 (x) | complete, 0 | same number stacked |
| enzocoralc_Tiny20_tiny20_xiao_tiny20_xiao | complete, 2 (x) | complete, 0 | same number stacked |
| Franticware_usb-to-ps1-mouse-pro_kicad_old_rev_PCB_1_usb2ps1mous | complete, 1 (x) | complete, 0 | copper graphic |
| freerouting_freerouting_scripts_benchmark_fixtures_PCBench_kitsp | complete, 0 | complete, 0 | - |
| freerouting_freerouting_scripts_benchmark_fixtures_PCBench_kitsp | complete, 2 (x) | complete, 0 | same number stacked |
| Heorenmaru_Hardware_Stm32Board_Small_oneSide_Stm32Board | complete, 1 (x) | complete, 0 | pour zone |
| Huaqiu-Electronics_kicad_qa_data_pcbnew_issue11814 | complete, 2 (x) | complete, 0 | same number stacked |
| jo5huajohn_BNO085-PCB_BNO085 | complete, 1 (x) | complete, 0 | pour net |
| justbuchanan_oasis_pcb_fabricated-boards_ledboard-v0.3_kicad_led | complete, 5 (x) | complete, 0 | copper graphic |
| justbuchanan_oasis_pcb_ledboard_ledboard | complete, 5 (x) | complete, 0 | copper graphic |
| MAHESH-THILAK-K_ADDITIONAL-PROJECT_VoltForge_Nexus_OTHER_FILES_V | complete, 1 (x) | complete, 0 | other |
| MarkWestbroek_MusicBrain_hardware_schematics_musicbrain-adc8_mus | complete, 3 (x) | complete, 0 | pour net |
| MarkWestbroek_MusicBrain_hardware_schematics_musicbrain-adc8_rel | complete, 1 (x) | complete, 0 | pour net |
| mikesmitty_power-manifold_hardware_controller_controller | open 5, 8 | open 5, 8 | other |
| mothenjoyer69_pcb-design-files_usb-uart-eqmod_usb-uart-eqmod_usb | complete, 1 (x) | complete, 0 | pour zone |
| OpenAir-Lab_capstone-project_Hardware_KiCAD_rf-amplifier-modulin | complete, 0 | complete, 0 | - |
| PantsForBirds_adsbee_kicad_prototyping_panel_proto_lna_board_pan | complete, 48 (x) | complete, 0 | same number stacked |
| PantsForBirds_adsbee_kicad_prototyping_panel_saw_eval_board_pane | complete, 48 (x) | complete, 0 | same number stacked |
| Qeteshpony_ESPFanController_pcb_ESPFanController | complete, 2 (x) | complete, 0 | copper graphic |
| robotique-udes_PCB_exo_ACE_2023_PCB_BioGenius | complete, 2 (x) | complete, 2 (x) | pour net |
| ryanhaus_rowan-baja-daq_accelerometer-board_pcb_accelerometer-bo | complete, 0 | complete, 0 | - |
| samienr_kryptonite_kryptonite | complete, 2 (x) | complete, 0 | same number stacked |
| smkent_pcb_pcb-archived_jigs_dip-adapters_dip-adapters | complete, 20 (x) | complete, 0 | numeric net name |
| smkent_pcb_pcb-archived_power_eta-buck_eta-buck | complete, 1 (x) | complete, 0 | pour zone |
| smkent_pcb_pcb-archived_stackable2_esp-cwww_esp-cwww | complete, 1 (x) | complete, 0 | pour zone |
| smkent_pcb_pcb_power_microbuck_microbuck | complete, 2 (x) | complete, 0 | pour zone |
| sommerper_framework16-keyboard-v1_kicad_framework16v1_framework1 | complete, 68 (x) | complete, 0 | same number stacked |
| Soraxem_trance_devboard_pcb_devboard | complete, 11 (x) | complete, 0 | numeric net name |
| Soraxem_trance_usb-dmx_pcb_usb-dmx | complete, 10 (x) | complete, 0 | numeric net name |
| Squantor_various_PCB_electronics_DIM_powergate_SOIC-8_DIM_powerg | complete, 0 | complete, 0 | - |
| Squantor_various_PCB_electronics_DIM_SN6505_PushPullConv_DIM_SN6 | complete, 1 (x) | complete, 0 | pour zone |
| swayerloren_KICAD_ENGINE_32_OPEN_KICAD_SAMPLE_INTAKE_normalized_ | complete, 1 (x) | complete, 0 | pour net |
| tdunning_low-pass_low-pass-2-sides_low-pass | complete, 2 (x) | complete, 0 | pour zone |
| tdunning_low-pass_low-pass_low-pass | complete, 2 (x) | complete, 0 | pour zone |
| urichess_chessboard_kicad_projects_others_CHESS_PIECES_DETECTION | complete, 4 (x) | complete, 0 | copper graphic |
| urichess_chessboard_kicad_projects_others_CHESS_PIECES_DETECTION | complete, 4 (x) | complete, 0 | copper graphic |
| urichess_chessboard_kicad_projects_others_READER_READER | complete, 3 (x) | complete, 0 | copper graphic |
| zOnlyKroks_pcb-consortium_Misc_7-seg-clock_7-seg-clock | complete, 0 | complete, 0 | - |

The two that still disagree: robotique's BioGenius (a dimension drawn
on B.Cu, which KiCad's fill keeps clear of and the lowering does not
lower: a band across the board our pour fills; the no-net copper of the
lowering-gaps package) and KiCad's own QA board issue6284 (a spoke joins
U102.4 and U102.6, which KiCad keeps apart). The 127 boards without a
licence were not routed again (run.py does not fetch them); by cause
they are the same mix as the 65 (table above).

**Expansion tier** (46 boards, 8 x 1, `--no-lock`; build/expansion-x1343
against build/expansion-x1322; build/expansion-x1344 is x1343's row for
row): 22 rows identical; passes 14 to 15
(strict 14 to 15), unconnected 787 to 758, new errors 18 to 20 (strict
15 to 16), vias 4022 to 4032; compare.py: better 7, same 37, worse 2.
Every changed row comes from one of the changes, found with a binary
whose changes could be switched off one at a time (build/xtog-*, not
committed) and from the boards' features:

- the new terminals: pads of one number on two sides on VTC-WISDOM's
  node (14 pads; 35 to 33 unconnected, 69 to 98 vias), ZSWatch (10; its
  starved thermal gone: a pass), ducktop2's bms (2), Wonkle (2),
  stellarcorp's magnetorquer (8; 86/86, a pass, to 84/86: the ladder kept
  its connect rung, I2C3.SDA and Net-(U8-LX1) open, KiCad 2 unconnected,
  so honest), mincrmatt12's MSign (2, with the brush below: 177/188 and
  15 unconnected to 165/188 and 60), TokayEdge (8; vias), LC_PAMV4b (2;
  a pass to one starved thermal, 51/52 either way) and pdlc (2); filled
  graphics named by code now terminals on bread-modular's noise (vias,
  length), minuet's light (22/23 to 23/23, a pass), pdlc (16/33 to
  21/33, 21 to 12 unconnected) and CleverHand's EMG_DAQ (21/22 to 22/22,
  a pass);
- the pour masks' brush (in the outline, clear of other zones): iorodeo's
  driver (170 to 109 unconnected), RS485M (2 to 1 unconnected, two
  starved thermals), Zulassungskarte (2 to 1), and in vias, length or
  work only Nervous-ECG, PCBMints, brickpico, OpenRX-Mono, AMLing2,
  FUEL4EP, anthracite, mikesmitty's backplane and nrf24_remote.

**Short tier** (tasks v7, 4 x 2, `--no-lock` while the designer tier held
the lock; build/short-x1344 against build/short-x1322, no row cut): 3 rows
identical (USB_Keypad, MIDAS, CharlieBoard), passes 7 to 7 (strict 7 to
6), unconnected 19 to 18, vias 1125 to 1116, work 2517 to 3080 s.
compare.py: better 1 (WLED_Matrix 101/104 to 100/104, 6 to 5
unconnected), same 11. The 9 changed rows, with both brush bands
switched off (build/stog-NOBANDS): back to x1322's on reCamera (vias,
length, first-route open 5 to 7), 0xCB-1337 (a starved thermal: no
longer a strict pass), USB_Cable_Tester (128 to 95 vias),
spell_tome_bottom (work 30.1 to 30.0 s) and framework_mobo_lefthalf
(vias, length); still x1344's on eternal-keypad (200 pads of one number
on two sides, its reversible switch footprints: 65 to 66 vias, a starved
thermal) and urchin (136 such pads: 42 to 65 vias); WLED_Matrix (two
filled rectangles named by code, now terminals) and OpenFC (12 custom
pads and solid zones: the custom pads' spokes and the solid reach, which
the switches do not cover) neither.

**Fingerprints** (route mode from the v16 layouts, quarter budget, 2 x 4,
`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, build/fp-conn-x1343
against build/fp-lower-x1322; build/fp-conn-x1344 has x1343's
fingerprints on all four): Castor and k30-SBC identical at every
phase; jetson identical up to the clean-up of its tracks attempt, which
the pour masks' brush changes (with both bands switched off every
fingerprint is x1322's), the board identical (324/340, 620 vias, 10791.4
mm, KiCad 19 unconnected); Sisu changes from the start: U2's pad 11 (GND)
on F.Cu and again on B.Cu at one place is two terminals now (183/192 to
182/192, KiCad 13 unconnected both, solder_mask_bridge 15 to 14).

**Gates**: `bash scripts/check.sh` and the clippy gate pass.

## Committed on 2026-10-22 (lowering gaps)

The two rules agents named KiCad findings the router could not see: the
edge clearance from NPTH holes and Margin graphics, copper graphics of no
net, and stubs of two nets leaving neighbouring pads. This closes them
and four more gaps the remaining findings showed, all in the lowering
(`board_router/lower.rs`, `geometry.rs`, `roundrect_pad.rs`,
`project_rules.rs`) except the stub check (`pcb-router`: `verify.rs`,
`router/finish.rs`); docs/router.md, "Board edges and copper of no net".

Binaries: x1322 is main before (ffa49a5); x1330 and x1331 steps on the
way (x1331: the edge clearance, copper of no net, footprint graphics and
texts, the stub check); x1332 all of it, measured; x1333 the commit (a
tuple of the lowering made a struct for clippy: the 20 rules boards
identical to x1332, rows and boards).

**What changed.**

- **Edge clearance from unplated slots.** KiCad 10.0.7's
  copper_edge_clearance takes an NPTH pad whose drill sizes differ (a
  milled slot) for board edge; a round NPTH hole, an oval drill of equal
  sizes and a plated slot keep only the hole clearance (kicad-cli on a
  board made for it: "NPTH pad of H2" at 0.2 mm, nothing for the others).
  The slot's hole obstacle takes the edge clearance as its floor (tracks
  and vias). buganini's USB-C shell slots (4 findings) and facade60's
  switch slots (3).
- **Margin graphics.** A graphic on Margin, on the board or in a
  footprint, is board edge to KiCad with its stroke and fill (a text on
  Margin is not): a keepout on every copper layer at the edge clearance
  (`margin_obstacles`; the zone filler knocks Margin out too). michpro's
  "Segment on Margin" (1).
- **Copper of no net keeps the Default class.** Board graphics, footprint
  copper graphics and footprint texts on copper of no net carry Default's
  clearance as their floor (a graphic whose net is not routed, that net's
  class): KiCad keeps the larger of both items' clearances, the router
  kept only the routed net's.
- **Footprint graphics on several layers.** KiCad 9 draws a footprint
  graphic on `(layers "B.Cu" "B.Mask")`; only `(layer ...)` was read, so
  tuxkeychain's GRAPHIC3 logo was neither copper nor a mask opening (10
  findings, tracks and vias 0.01 to 0.07 mm from it). Footprint graphics
  now lower on every copper layer they name and open every mask layer.
- **Footprint texts on copper and mask layers.** A visible `fp_text` or
  `property` on a copper layer is copper of no net, on a mask layer an
  opening (`footprint_text_box`: the stroke font's box in the
  footprint's frame; KiCad 7 and 8 store the angle relative to the
  footprint, checked with kicad-cli; knockout texts their plate, hidden
  ones nothing). The intake's jnftech test points (Value fields on F.Cu:
  clearance and shorting_items) and Tert2's Reference on a mask layer.
- **Pad-centre stubs keep the rules.** The cosmetic stub from a route's
  node inside a pad to the pad's centre is drawn at the track's width; on
  a pad narrower than the track it reaches past the pad. It is checked
  against fixed copper when laid and against routed copper when the
  routes are emitted, where `finish` drops a stub too close; that check
  (`verify::verify_with_rules`, the classes plus `ClassRules`' pair
  clearances per item kind and layer; `verify` itself unchanged) now
  holds the rules. tuxkeychain's three "Minimum Trace Spacing (outer
  layer)" findings: GND's 0.3 mm stubs along U4's 0.2 mm QFN pad 2,
  0.104 mm from HPL's track and stub under 0.127. Measured alone (x1322
  with only this change, tuxkeychain): the same routes less three stub
  segments, clearance 13 to 10 (the GRAPHIC3 ten), 5 unconnected both.
- **Arcs.** A copper (or mask, or Margin) arc graphic is flattened to
  chords up to 0.005 mm inside the arc; its stroke now grows by that
  tolerance. name-card's antenna AE1 (a 10.7 mm arc): a track 0.1978 mm
  from it under 0.2 (2 findings).
- **Zero chamfers are sharp corners.** A rounded-rectangle pad's corner
  named in `(chamfer ...)` with `chamfer_ratio 0` is sharp in KiCad (not
  rounded; kicad-cli: 0.1546 mm from the corner, 0.32 from a rounded
  one); the lowering rounded it. AQEYE's solder jumpers JP1 and JP2 (2
  findings, tracks 0.126 and 0.112 mm from the inner corners).
- **Net keys of the project rules.** `resolve_project_rules` keyed the
  connections before noting the board's bare net names: on a board with
  both `/VIN_CELL` and `VIN_CELL` (meshsat) the rules were keyed
  `VIN_CELL` while the lowering asked for `/VIN_CELL` and fell back to
  Default's (0.25 mm tracks, 0.7 mm vias, 0.14 clearance against CELL's
  0.8, 1.0 and 0.15: 6 findings).

**Rules boards** (rules-tasks.json, route mode, 30 s, 8 x 1, `--no-lock`),
x1322 against x1332: 14 rows identical; passes 12 to 18 (strict 12 to
18), new errors 18 to 0 (strict 19 to 1), unconnected 6 to 4, vias 493
to 464, work 233 to 230 s.

| board | gap | x1322 | x1332 |
| --- | --- | --- | --- |
| buganini Kikakuka gerber | NPTH slots as edge | 5/5, copper_edge_clearance 4 | 5/5, pass |
| facade60 main | NPTH slots as edge | 92/92, 98 vias, copper_edge_clearance 3 | 92/92, 69 vias, pass |
| michpro PY32F002 | Margin graphic | 12/12, copper_edge_clearance 1 | 12/12, pass |
| name-card | arc chord error | 13/13, clearance 2 | 13/13, pass (one track a node over) |
| AQEYE-SB-D1 | zero chamfer | 28/28, 23 vias, clearance 2 | 28/28, 25 vias, pass |
| meshsat pcb-d-aprs | net keys | 60/61, 2 unconnected, clearance 6 | 61/61, pass |

x1331 (without the last three) changed the first three rows only. The two
rows left failing are ChengminLi (the courtyard rule's dead jumper pads)
and kisurf (8 mm between pins of one part; strict starved_thermal 1),
both as on x1322.

**Expansion tier** (8 x 1, `--no-lock`), x1322 against x1332 (x1331 the
same): 45 rows identical; tuxkeychain 38/42 to 35/42, clearance 13 to 0,
unconnected 5 to 11, vias 69 to 66. Its GRAPHIC3 logo is B.Cu copper the
routes crossed within 0.01 mm before; routing around it at 30 s leaves
SDC0_D3, SDC0_D0, SPI1_MISO and SPI_MISO open where SPI1_CLK was (no dead
pads). Totals: unconnected 787 to 793, new errors 18 (strict 15) to 5
(strict 2), vias 4022 to 4019, 14 passes both. The new errors left are
starved_thermal on ZSWatch (1) and anthracite (4, strict 1): the pour
model, not the lowering.

**Fingerprints**: Castor, k30-SBC, jetson and Sisu, route mode from the
v16 layouts (`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`, 2 x
4, budget scale 0.25), x1322 against x1332: every phase fingerprint, the
board (md5) and the row identical. All four have copper graphics
(footprint polygons of no net on Castor, k30 and jetson, now at
Default's clearance, which no class there exceeds; Sisu's GND
rectangles and polygons), jetson two copper arcs, none near enough a
route to move it; none has a Margin graphic, an NPTH slot, a zero
chamfer, a footprint graphic on several layers or a visible footprint
text on copper or mask.

**Short tier** (tasks v7, 4 x 2, `--no-lock` while the designer tier
held the lock; results count work), x1322 against x1332: all 12 rows
identical (routed, vias, length, KiCad's findings, work), all 12 result
boards byte-identical, no row cut; 7 passes, 19 unconnected, 1125 vias
both. framework_mobo_lefthalf's one NPTH slot is the only item of these
gaps in the tier (Default is the smallest class on every short-tier
project, so copper of no net keeps what it kept); it moved nothing.

**Four intake boards with the text findings** (the intake's stripped
boards and router settings, route mode, 30 s of work, kicad-cli's DRC;
in the scratchpad, not a tier), x1322 against x1332: jnftech's SOIC
board clearance 1 and shorting_items 1 (Value fields) to none, 4
unconnected both; its UQFN board clearance 5 and shorting_items 2 to
none, 3 unconnected both (annular_width 6 both: the designer's); Tert2's
jogi solder_mask_bridge 11 (Reference fields on F.Mask) to 0,
unconnected 43 to 22 (the edge, hole and clearance findings left are
the designer's); stonedDiscord's ZLKalt shorting_items 1 to 0 but 0 to
4 unconnected: the router's pour-connect attempt now counted 0 open and
was kept without KiCad's check (the intake's settings), and KiCad's fill
of the B.Cu GND zone leaves four pieces apart (the pour model, not the
texts, which are on F.Cu).

**Lowering gaps in the intake's findings** (the 2026-10-18 intake's
result DRC on 1 725 boards, an older binary; findings of routed copper
against a fixed item, on boards with more of the type than the
designer's): closed here: copper_edge_clearance against NPTH slots (28
on 5 boards) and Margin graphics (31 on 5), footprint fields on copper
(clearance 8 and shorting_items 4 on jnftech's boards, shorting_items 1
on stonedDiscord's ZLKalt) and on a mask layer (solder_mask_bridge 16, Tert2).
Not lowering, closed before: clearance against pads and track_width
(the project rules), copper_edge_clearance against Edge.Cuts and
hole_to_hole against NPTH pads (rules' values, such as conorcurley5's
"Clearance: hole to hole (perimeter), different nets"). Left: dimensions on
copper (shorting_items 3 and clearance 1, two boards: KiCad counts a
dimension on a copper layer as copper), a multi-line copper text
(TomKeddie's supercon SAO, 3, not analysed), solder_mask_bridge against
pads and mask graphics on 11 boards (38 findings, after the 2026-10-15
mask model; not analysed here), track_dangling and
track_not_centered_on_via (the router's output, not the lowering).

Unit tests: board_router lowering_gap_tests (an NPTH slot, a round hole,
a plated slot, Margin on the board and in a footprint and a Margin text
against tracks through `verify`; a footprint polygon on B.Cu and B.Mask,
a board line of no net at Default's clearance against a smaller class,
a footprint Value on F.Cu at its turned position, a hidden field, a
footprint text on F.Mask), roundrect_pad (a zero chamfer is sharp),
project_rules (a hierarchical net beside a bare one keeps its class;
fails without the fix), pcb-router rule_tests (`verify_with_rules` holds
a track-to-track rule `verify` does not, not against a via). `bash
scripts/check.sh` and the clippy gate pass.

Logs: build/rules-x1331, -x1332, -x1333; build/expansion-x1331, -x1332,
build/expansion-lower-stubonly (the stub check alone); build/fp-lower-x1322,
-x1332; build/short-x1332.

## Committed on 2026-10-21 (ladder fault isolation)

Review finding 8: `route_kicad_board` (board_router/route.rs) was about
1 500 lines of closures over `Cell`s with no test of its control flow; its
worker threads were joined with `.expect`, and two asserts reachable from
the input (nets in 15 bits, layers at most 32) panicked, so a panic in one
probe or rung lost every board the ladder held (exit 4). And the finer
pitch's machine-memory gate read the process's peak resident size, which
follows how many probes ran side by side. Branch ladder-fault-isolation;
snapshots x1310 (main at b306347, the baseline), x1311, x1312 (this on
b306347), x1313 (on 535edb5), x1314 (on 5a99d89).

**The ladder as steps** (board_router/ladder.rs, a move). `Ladder` holds
the ladder's state (its work, the reserve of a layout's ladder and route
mode's, the work bound, the best board and attempt, the check gate, the
skipped steps, the failures) and runs its steps in order (`Ladder::run`):
`probes` (the rung probes side by side, their leader's continuation,
`continue_probe`), `rungs` (per pitch: `pitch_skipped` for the finer
pitches' four gates, `pitch` with `rungs_side_by_side`, `rung_wave`, the
last rung's `seed_retry`), `seed_rounds`, `steps` (narrow-signal, then
minimum clearance, each `step`). The closures became methods
(`spent`, `bound_left`, `rung_left`, `judge`, `step_has_time`, `blocked`,
...); `ladder_rungs` and `ladder_pitches` are the rung and lattice lists.
What routes, checks and measures is a `LadderRunner`: route.rs's
`KiCadRunner` (lowering, `Router::new`, `probe`, `resume`,
`write_attempt`, `route_kicad_board_once`, `check_attempt`, the memory
limits) and the scripted `Fake` of ladder_tests.rs. route.rs went from 2 146
to 800 lines (the preparation, the runner, the finish, `route_seeds`,
`core_config`, `finish_routed_board`); every log line and json field
stays.

**Fault isolation.** Every probe (its router's build too), the
continuation, every rung (in its thread), the rung's seed retry, every
seed round and both steps run under `catch_unwind` (`isolate`; a thread
join's error counts the same, `route_seeds` passes a seed thread's panic
on with its own message), and so does KiCad's judgement of each. A panic
is logged (`ladder: the rung pours=tracks pitch=regular failed (panic:
...); the ladder goes on with its best board`), listed in
`attempts_failed` of board-router.json (absent when empty) and in the
verdict's `next_actions` ("N attempts of the router's ladder failed ...:
the board is the best of the other attempts; report the panic"), and the
ladder goes on: a failed probe counts no work and leaves the lead to the
next best; a failed continuation leaves the probed rung to the regular
rungs; a failed rung or seed round leaves the best board. Only when no
attempt produced a board does the run end in an error, an internal one
(exit 4: "no attempt of the ladder produced a board: N failed (...)"),
where `expect("at least one routing attempt")` panicked before. The test
hook `PCB_MAKER_TEST_PANIC_ATTEMPT=<kind>[:<n>|:all]` (probe,
continuation, rung, seeds, step) makes the n-th attempt of a kind panic;
it is in `RESULT_ENVIRONMENT`. Not covered: a panic outside the attempts
(the preparation, the finish, the neck repair) still ends the run, and
main.rs's panic hook keeps the first panic's message, which may then be
an isolated attempt's.

**Capacity limits** (pcb-router grid.rs, build.rs). `check_capacity`
returns an error naming the limit and the board's count: more than
`MAX_NETS` (65 533) nets ("the board has N nets: the router names nets in
16 bits, at most 65533") or `MAX_LAYERS` (32) copper layers. The lowering
calls it and returns it as an input error (exit 2); `Router::try_new`
returns it; `Router::new` and the static maps keep it as an assert. The
net limit went from 15 bits (32 766 nets) to the 16 of the static maps:
the search records (`Hot`) hold `owner(net)` in a u16 and the bit kept
for a hard guard flag is unused since the router split (changelog,
"quality: pcb-router").

**Memory decisions.** The finer-pitch gate against 0.4 of MemTotal now
takes the router's estimate from the first attempt's maps
(`router_peak_mb`, resident) instead of the process's peak resident size
after the first rung (which included the probes side by side and, in a
layout, the first route); the probes' first router no longer falls back
to the growth of the resident size when its memory report has no maps.
No ladder decision reads the peak any more, so no `cut` kind is needed;
how many routers run side by side still follows what is free now
(MemAvailable, the cgroup's use), which by design changes no result. The
layout's placement race (board_layout.rs, not this package) sizes its
side-by-side count from `memory_mb()`'s MemAvailable the same way.

**Folded in from the thermals package.** The kept attempt's
`island_stubs` (pcb-router's `RoutingResult::island_stubs`) is in
board-router.json and the verdict's `repairs.island_stubs` (was `null`);
`PCB_ROUTER_ISLAND_STUBS` is in `RESULT_ENVIRONMENT` in place of
`PCB_ROUTER_NO_ISLAND_STUBS`. The lowering records each pad's zone
connection by obstacle index beside the label lists
(`Board::solid_pad_obstacles`, `isolated_pad_obstacles`: the pad's own
`zone_connect`, else its footprint's), so that 74th's J3.S1 (SMD shield
pads without a zone connection, through-hole ones with the zone's) can be
told apart; the router does not read them yet (`kicad_spokes`,
`thermal_ring_pads`, `prepare_net` are the follow-up).

**Tests.** ladder_tests.rs (11, the scripted runner): the rung and
lattice lists; the hook's parsing and `isolate`; four probes sharing half
the budget, the leader continuing with at most half of what is left and
its deadline at the bound, a complete continuation ending the ladder; a
panic in one probe (the hook and a router panic) recorded and the next
best leading; a panicked continuation falling back to all rungs; route
mode's first rung with all that is left, its work kept for the
narrow-signal step, the next rung's share, the seed retry, the step last
with what was kept, every router with a work deadline and the work within
the bound; a panic in a rung that runs beside others keeping the best
board and no rung directory behind; every attempt failing leaving no
board; a panic in the seeds keeping the rung's board; a layout's ladder
keeping 300 s for the step with the next rung's deadline where the
reserve begins; a finer pitch skipped by its maps against a small
machine. grid.rs: the capacity limits. tests.rs: each pad's zone
connection. cli_exit_codes.rs: `PCB_MAKER_TEST_PANIC_ATTEMPT=rung` on
the three-pad board (the regular rung panics, the 0.075 mm rung's board
is kept, exit 1 as without KiCad's check, `attempts_failed` and the
next action), `rung:all` (exit 4, "no attempt of the ladder produced a
board: 3 failed"). The probe hook through the binary needs a board of four
layers and 120 connections; the scripted runner tests it.

**Identity.** Route mode on the v16 placements of Castor, k30-SBC,
jetson and Sisu (`PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`,
4 threads), x1310 against x1312: the same board (md5), the same
fingerprints (4, 6, 29, 26), vias (159, 560, 615, 410) and work (310.7,
448.1, 1219.0, 989.5 s) on every board; the rows differ only in
`repairs.island_stubs` (null to 0). Short tier (tasks v6, under the lock)
x1310 against x1312: every row identical apart from timing and
`repairs.island_stubs`, all 12 result boards byte-identical (19
unconnected, 1125 vias, 2517 s of work, 7 passes; against short-x1170
main itself moved: 1134 to 1125 vias, 2501 to 2517 s, the thermals
package). Expansion tier (30 s, 8 x 1) x1310 against x1312: all 46 rows
identical apart from `routing_seconds` and `repairs` (807 unconnected,
4038 vias, 1279 s of work, 14 passes, 0 cut). Rebased on 5a99d89 (the
runtime calibration, project rules), x1277 (main) against x1314 with
`PCB_WORK_RATE=3.15e6,150e6`: the same four fingerprint runs identical
(boards, fingerprints, vias, work; rows apart from `repairs` and the
measured rates and work/wall ratio), the short tier identical (all 12
boards byte-identical, 2517 s of work, 7 passes) and the expansion tier
identical on all 46 rows (787 unconnected, 4022 vias, 1275 s, 14 passes,
0 cut) apart from timing and `repairs`. Logs in build/ladder3 (qb-, qb2-,
short-, short2-, expansion-, expansion2-).

## Committed on 2026-10-21 (router rule features)

The project rules work (below) lowered the project's rules but left
three kinds the router could not keep: width floors per net, clearances
between two sets of nets, per item type and per layer, and the hole to
hole spacing within a net. This adds them to the router
(`crates/pcb-router/src/rules.rs`, `Config::net_rules`) and to the
lowering (`project_rules.rs`, `project_rules/dru.rs`); docs/router.md,
"Project rules" and "Rules beyond the classes".

Binaries: x1277 is main before (5a99d89; main's x1315 after the ladder
refactor routes identically), x1320 this work on 98efa4a, x1322 the
commit (rebased on 6f10989, with the two changes the expansion tier
asked for, below). x1321 was not measured.

**What changed.**

- `pcb_router::NetRules` (`Config::net_rules`, `None` by default):
  `width_floor` per net name, `clearances` (`ClearanceRule`: two sides
  as net names or every item, the pairs of item kinds as bits of track
  or via against track, via or pad, the layers, the clearance) and
  `same_net_hole_to_hole`. `KiCadBoardRouterConfig::net_rules` (a skipped
  field, one line in board_router/config.rs) carries it from
  `resolve_project_rules` to `core_config` (one line in
  board_router/route.rs); main.rs's file merge copies it.
- The router splits its rule classes by group (the nets no rule tells
  apart) and width floor, keeping a class's index for the first nets
  that need no other: rules that split nothing route byte for byte as
  without them. Stamps are built per pair of classes and per layer
  variant (one without layer-qualified rules); static maps keep pads,
  existing tracks and vias at the rules' clearance per layer; pad stubs
  (`stub_is_clear`) too. A net's neck class is its floor where that is
  wider than the board's neck; stubs and escapes take it.
- Same-net hole to hole: where the project rates hole_to_hole an error,
  a via keeps the board's hole to hole from its own net's pad holes.
- The lowering evaluates a clearance rule per pair of net groups, pair
  of item kinds and layer; a rule true everywhere for a set that raising
  covers stays a per-net clearance, anything else becomes a pair rule
  (`applied`; `approximated` where it tells SMD from plated pads). A
  per-net `track_width` minimum floors that net's neck-downs and narrow
  width (`applied` where it was `approximated`).
- KiCad applies the last matching rule: a clearance, hole clearance,
  edge clearance or hole-to-hole rule no longer counts where a later rule
  with a minimum of the same constraint surely matches the pair (the
  groups tell apart what the later conditions do). JLCPCB's template
  ends with "Min. Spacing" 0.127 mm on everything, after its "Pad to
  Track" 0.2 mm: KiCad checks 0.127 between pads and tracks (AMLing2's
  drone carrier, where KiCad found none of the 0.2), and so does the
  router now (`no_effect`, "a later rule takes every pair it matches").
- A pad keeps what tracks of its net keep from other nets' copper too:
  its net leaves it by a stub off the lattice that only the pad's
  clearance protects (x1320's tuxkeychain: 0.127 mm between outer-layer
  tracks over classes of 0.1 left stubs 0.104 mm from other tracks).

**Route mode on the 20 rules boards** (rules-tasks.json, 30 s, 8 x 1),
x1277 against x1322 (x1320 the same rows): 17 rows identical (routed,
vias, length, KiCad's findings, work); passes 11 to 12, new errors
(strict) 38 to 19, unconnected 2 to 6, vias 494 to 493.

| board | rule | x1277 | x1322 |
| --- | --- | --- | --- |
| nuclone (Squantor) | hole to hole 0.5 (error), same net | 18/18, hole_to_hole 2 | 18/18, pass |
| ChengminLi flyback | 2.5 mm primary to secondary, pair rule | 38/38, clearance 1 | 35/38, 2 unconnected, no error |
| kisurf issue24211 | 8 mm feedback to switch node, pair rule | 34/34, clearance 11 (strict 17) | 32/34, 2 unconnected, no clearance (strict: starved_thermal 1) |
| the other 17 | | | identical |

ChengminLi's two open connections are the pins of the Y capacitor's
jumper YPS (primary PSY1, PSY2 and secondary SGND, 2.54 mm apart): its
file loosens the isolation there by a later rule, "YPS jumper pins
(intended bridge)", 0.5 mm between items inside YPS's courtyard, which
the router cannot evaluate (`intersectsCourtyard`): it keeps 2.5 mm and
no track reaches those pads (diagnostics.dead_pads names them). kisurf's
rule asks 8 mm between the feedback and the switch node, whose pins sit
on one part (U4.2 and U4.8, 3 mm apart): no track can leave either pin
and keep it; the designer's own board breaks it (10 findings). Both are
dead pads of the rule, not of the router; the earlier per-set raise
(x1270) had left 19 and 8 open there.

**Expansion tier** (x1277 against x1322, 8 x 1, no row cut): 45 rows
identical, unconnected 787 to 787, new errors (strict) 17 to 15, vias
4022 to 4022, 14 passes both. tuxkeychain: 37/42 to 38/42 routed, 5
unconnected both, clearance 15 to 13: the 9 "Pad to Track clearance"
findings (vias 0.187 mm from tracks under 0.2, the rule matches plated
items: vias here) are gone; 3 tracks of GND and HPL 0.104 mm apart
where both leave neighbouring pads by stubs under the outer-layer 0.127
rule, and 10 (was 6) tracks and vias within 0.1 of a copper graphic
polygon without a net (GRAPHIC3: the lowering's graphics, not a rule)
remain. AMLing2 (x1320 without the later-rule change: 10 to 30
unconnected, the "Pad to Track" 0.2 kept) is identical to x1277.

**Short tier** (x1277 against x1320 and x1322, tasks v7, 4 x 2): all
12 rows identical on both; no short-tier project has a .kicad_dru rule.

**Fingerprints**: Castor, k30-SBC and jetson (projects rating hole to
hole an error, so their rules hold the same-net spacing and split
nothing) and Sisu, route mode from the v16 layouts with
`PCB_ROUTER_FINGERPRINT`, 30 s: every phase fingerprint of x1320 and
x1322 equals x1277's.

**Anthracite** (the coordinator's question on the project rules
change): GNDPWR is in `power` (explicitly, priority 4) and `GND` (by
pattern, priority 0); KiCad's composite class takes GND's width 0.2 and
via 0.6, as the router does since x1275 (the clearance is 0.15 in
both). A class width is no minimum in KiCad's DRC, so the narrow width
need not keep 0.5. The thermal spokes follow the zone, not the class:
the spoke model draws them at the zone's thermal_bridge_width
(`thermal_reach - thermal_gap`) and drops those narrower than its
min_thickness (`kicad_spokes`). starved_thermal 1 to 4 came with 2 to
0 unconnected: different pads (J107, J106, R503, R703 against J104),
other routing.

**What stays unmodelled**: rules on areas, courtyards, footprints,
component classes (their conditions are unknown), also where a later
such rule loosens an earlier one (ChengminLi's jumpers); creepage,
lengths, skew, diff pairs, physical clearances, disallows other than
blind, buried and micro vias; the edge clearance from NPTH holes and
Margin graphics; pair rules in the pour model (fills keep the classes'
clearance from other nets' tracks) and in the internal verifier (KiCad
checks them); stubs of two nets leaving neighbouring pads are not
checked against each other (tuxkeychain's 3).

Unit tests: pcb-router `router::rule_tests` (a pad-to-track rule keeps
tracks from pads only, a rule between two sets splits their classes with
stamps per layer variant, a width floor holds for the neck, the same-net
hole spacing blocks a via the copper rule allows, rules that split
nothing route identically); project_rules::dru (pair rules between two
classes, item types and layers as pair rules on two and four layers,
SMD pads approximated, the last matching rule); project_rules (a per-net
width floor, a project with an isolation rule, JLCPCB's pad to track
and an outer-layer via rule, hole_near_hole as an error). `bash
scripts/check.sh` and the clippy gate pass.

Logs: build/rules-x1277, -x1320, -x1322; build/expansion-x1277, -x1320,
-x1322; build/short-x1277, -x1320, -x1322. x1322's tiers ran with
`--no-lock` while the designer tier (x1315) held the lock for hours
(results count work, not wall time).

## Committed on 2026-10-21 (config reference and stale docs)

Review finding 10: the agent-facing surface is about 130 config keys and
about 50 environment variables, most keys named in no document, and the
documents carried stale statements.

**The reference is generated.** `tools/config-reference` (a workspace
crate; `cargo run -p config-reference`) writes
[config-reference.md](config-reference.md) from the config structs: the
field names, types, doc comments and serde attributes from their
sources, the defaults from the structs at run time (a settings file of
`{}` read through serde and serialized again; the core configs'
`Default` printed with `{:#?}`; a default read from the environment, as
the router's `island_stubs`, shown with and without its variable). One
table each for the router settings (with `topological`, `pin_swap`,
`add_pours`, `net_classes` and the connection rules), the router core
(`pcb_router::Config`, with the settings `core_config` copies into each
field, read from that function), the placer settings, the layout
settings (with `swappable`), the placer core (`Config`, `global`,
`anneal`, with what `place_kicad_board` copies), and, as an output,
board-router.json's `rules` (the project-rules report). Each key is
marked when docs/router.md or docs/placer.md names it in code, and
internal (`#[serde(skip)]`) fields are marked as such. The same tool
writes the hand-over's environment index ("Switches", one line per
variable, grouped user-facing, switches, diagnostics, tests): every
`PCB_*` string literal of the crates' sources outside tests needs its
line in the tool's `ENVIRONMENT`, a line whose variable the code no
longer names is an error, and every user-facing variable must be in
`pcb-maker --help` (`ENVIRONMENT_HELP` in src/main.rs).
`tests/config_reference.rs` (in scripts/check.sh) fails while either
file differs from what the tool makes of the tree, or while one of those
checks fails; it names the first differing line.

Counts (13 structs, 249 fields; the three report structs aside): 155
file keys, 12 internal (serde-skipped: 10 router settings, 2 placer
settings), 82 fields of the core configs. With a doc comment: 194 of 249
before, 233 now; the other 16 are in crates/pcb-kicad/src/board_router/
config.rs (14 router settings, 2 of `topological`), which the ladder
fault isolation package has open: their comments are written and wait
as a patch for after it lands. Named in router.md or placer.md: 35 of
the 155 file keys (unchanged; the reference now documents every key).
No field is unused: each is read somewhere outside its definition.

**Doc comments written** for every field that had none (from the use
sites): router core `present_factor`, `present_growth`,
`history_increment`, `max_iterations`, `via_reduction_factor`,
`verbose`; placer settings `keep_rotation`, `pins_per_halo_track`,
`maximum_halo_mm`, `seed`; layout `placer`, `pin_swap`,
`congestion_candidates`, `congestion_probes`, `congestion_score` and
`swappable[].part`; `net_classes[]` `name` and its four sizes; the
connection rules' four sizes; `pin_swap.seed`; placer core `global`,
`anneal`, `refine_passes`, `global.bins`, `stop_overflow`,
`max_iterations`, `frame_interval`, `seed`, `routing_layers`,
`track_pitch`, `anneal.stages`, `moves_per_part`, `seed`.

**Stale statements fixed** (no behaviour change except the verdict's
environment echo and the KiCad version warning):

- router `stall_drop`: the doc called 0 pathological while 0 is the
  default. It now says what the code does: 0 turns the threshold off
  (any new low counts, as below the cap), the final ladder keeps it, and
  the layout's in-place router (first route and moves) and placement race
  set 0.1 unless the router settings give `stall_drop`.
- router `seed` carried `verbose`'s first line; each has its own now.
- `pcb-maker --help` listed five variables; it lists the fourteen
  user-facing ones (added `PCB_WORK_RATE`, `PCB_KICAD_TIMEOUT`,
  `PCB_MIN_CONNECTION_FLOOR`, `PCB_KICAD_NO_NECK_REPAIR`,
  `PCB_ROUTER_ISLAND_STUBS`, `PCB_MAKER_CACHE_DIR`, `PCB_MAKER_NO_CACHE`,
  `PCB_MAKER_KICAD_USER_LIBS`, `PCB_MAKER_MEMORY_MB`).
- The verdict's `environment` echoed `PCB_ROUTER_NO_ISLAND_STUBS`, which
  nothing reads since the island stubs went off; it echoes
  `PCB_ROUTER_ISLAND_STUBS` (verdict.rs, docs/agents.md). The hand-over's
  switch list said the same and named two test fixtures the code no
  longer has.
- docs/router.md's limit "Two copper layers, through vias only": inner
  layers are routed; the limit is through vias only.
- docs/handover.md: the leftover "Fill in when it lands: v17" template
  and its fragment, replaced by the v17 result it was waiting for; the
  open problem listing `bus_rows`, `supply_via_room` and
  `replicate_channels` as built and off (removed on 2026-10-16).
- docs/constraints.md documented `bus_rows`, which the placer config
  rejects since 2026-10-16; the paragraph also called `keep_on_pieces`
  opt-in while it is on by default.
- docs/changelog.md: seven sections out of date order (2026-10-16 and
  2026-10-20 among the 2026-10-21 ones) moved, sections verbatim (two
  got the blank line before the next heading they lacked).
- README and `--help`: "KiCad 9 or newer", while the import writes KiCad
  10 boards and nothing ran on KiCad 9: KiCad 10 (checked with
  `kicad-cli version` 10.0.7). native_report.rs asks `kicad-cli version`
  (cached, as the DRC cache does) before the first KiCad check of a
  process and warns once on stderr when it is older than 10.

**The flaky expansion test.**
benchmarks/expansion/test_triage.py's
`test_timeout_kills_descendants_and_keeps_cpu_measurement` failed about
one run in three under load (6 of 15 runs at load 18 here). The
traceback caught: `ProcessLookupError` from reading `/proc/<pid>/stat` of the descendant
after `exists()`: the kill worked, and the process was reaped between
the look and the read (ESRCH). A second race waited behind it: the
0.4 s wall timeout could pass before the parent, under `nice -n 19`
and `prlimit`, had started the descendant and written its pid. The test
now runs `run_process` on a clock of its own that passes the timeout
only once the pid file (renamed into place whole) exists, counts a
descendant gone on either error, and bounds its waits at 300 s where
both processes would sleep 600 s: 20 of 20 runs passed at load about 20.
benchmarks/expansion's tests are now in scripts/check.sh (41 pass).

## Committed on 2026-10-21 (project rules at scale)

The intake (docs/benchmarks.md, "Expansion intake") showed the router
ignoring rules KiCad enforces: track_width on 89 boards (84 on inferred
rules: KiCad's 0.2 mm minimum where the project states none, against our
0.127 neck), clearance from `.kicad_dru` rules (1 014 findings) and from
net classes (465, all `HIGH_V` on dennisss's dacha boards), edge
clearance from the board setup (203) and custom rules (40), hole to hole.
`project_rules.rs` now reads the project as KiCad 10.0.7's DRC does and
`project_rules/dru.rs` reads the custom rules (docs/router.md, "Project
rules").

Binaries: x1274 is main at b306347, x1275 this work on it, x1276 the
commit (rebased on the research removal, 0dff44e). Snapshots x1270-x1273
are the steps on the way (x1270 and x1271 on 0d13e80).

**Census** (`experiments/project-rules/census.py` over the 5 834 intake
boards; `census_of_custom_rules`, an ignored test, runs the lowering over
the 316 with a `.kicad_dru`). Projects: 390 boards have no `.kicad_pro`,
30 one without design rules, 33 one without `min_track_width` (all take
KiCad's 0.2); 842 boards have a pattern that is not a plain wildcard, 133
(399 patterns) a real regular expression (`.*(_TAP|V_POE|POE_RAW).*`);
105 a class without a clearance or width (KiCad 8, Default's). Custom
rules: 316 files, 76 with no rule, 4 040 rules on 240 boards.
Constraints by boards: clearance 193, hole_clearance 142, track_width
140, hole_size 137 (+96 with a max), annular_width 126, edge_clearance
125, hole_to_hole 98, silk 97, text 92, via_diameter 84,
physical_hole_clearance 21, courtyard 10, diff_pair_gap 8, creepage 6,
disallow 5+, length 3, skew 2. Condition atoms by boards: `Type` 128,
`isPlated()` 97, `A.Net != B.Net` 97, `Fabrication_Property` 91,
`Pad_Type` 70, `Layer` 49+15, `NetClass` 36, `Hole_Size_X` 24,
`memberOfFootprint()` 15, `hasNetclass()` 12, `NetName` 10, `Reference`
9, `insideArea()` 8 (1 371 uses), courtyards 6+6, `inDiffPair()` 4;
`(layer outer)` 96, `inner` 92, a named layer 114. Most files are fab
templates (JLCPCB, PCBWay).

What the lowering made of them (rules / boards): clearance applied 4/3,
approximated 3/2, no effect 387/156 (looser than the classes), skipped
106/62 (severity, or an inner layer on two layers), unmodelled 95/44
(pair rules, pad-to-track rules, areas);
track_width applied 4/4, approximated 12/7, no effect 185/139,
unmodelled 1 370/7 (one generated file of `insideArea` necks);
edge_clearance applied or approximated 20/20, no effect 85/84;
hole_clearance approximated 168/78 (JLCPCB's 0.33 PTH to track over the
0.25 setup), no effect 111/80; hole_to_hole approximated 105/51 (0.5
between nets; most projects keep KiCad 8's `hole_near_hole: error`),
skipped 103/40 (warning); hole_size, via_diameter and annular_width
nearly all no effect (the classes' vias are larger). Reported only:
creepage 8/6, diff_pair_gap 163/8, length 30/5, skew 7/5,
physical_clearance 6/3, physical_hole_clearance 40/21, disallow 22/7
(tracks or vias in areas), rules on areas, courtyards, references or
component classes. Silk, text, courtyard, thermal and assertion rules
are `not_routing`.

**What changed.**

- Board setup minimums the project does not state take KiCad's defaults
  (min_track_width 0.2, via diameter 0.5, through hole 0.3, annular ring
  0.1; edge 0.5 and holes 0.25 as before), so the neck-down is 0.2
  there. `rules.kicad_defaults` lists them.
- Net class patterns match as a wildcard or an anchored regular
  expression (regex-lite, a new dependency of pcb-kicad); a net takes
  every class it matches, each value from the first by priority that has
  it, else Default's; a class's missing value is Default's (was KiCad's
  built-in 0.2/0.6/0.3).
- `.kicad_dru` rules lower to per-net clearances, widths (min, max, opt)
  and vias and to the board's edge, hole and hole-to-hole clearances,
  the largest of every rule that can match (KiCad takes the last match;
  the largest is never less). Pair rules between two sets of nets,
  clearances from pads or vias to tracks that match no two tracks,
  rules on what the router cannot evaluate and creepage, length, skew,
  diff pair, physical clearance and disallow rules are reported, not
  kept. Every rule read and what became of it is in `board-router.json`'s
  `rules` (`custom_rules_file`, `custom_rules[]` with rule, constraint,
  status, detail; `unmodelled[]`); the route and layout verdicts' next
  actions name the unmodelled rules when the result is incomplete, and
  `drc-kicad-board` prints them on a `next:` line when KiCad finds errors.
- Plumbing outside the rules: `KiCadBoardRouterConfig::project_rules`
  (a skipped field, board_router/config.rs), `KiCadRulesSummary` flattens
  it (board_router/result.rs, one field and one line in `of`), main.rs's
  file merge copies it, `KiCadRunFacts::unmodelled_rules`.

Two over-approximations were measured and dropped. x1270 lowered a rule
between two sets of nets by raising one set against every net:
ChengminLi's flyback (2.5 mm primary to secondary) went from 1 finding
to 19 unconnected, kisurf's issue24211 (8 mm feedback to switch node)
from 11 findings to 8 unconnected (route mode, 30 s, against x1225).
x1273 still raised every net for JLCPCB's "Pad to Track" (0.2 mm,
`A.Type == 'Pad' && B.Type == 'Track'`): AMLing2's drone carrier went
from 10 to 35 unconnected and tuxkeychain from 23 to 41 (expansion tier
against x1261); routed with that rule removed from the file, AMLing2
had 10 unconnected and no error. Both kinds are reported now; a pad
clearance floor in the lowering would keep the second exactly.

**Route mode on 20 intake boards** (`benchmarks/agent-tasks/rules-tasks.json`,
from `experiments/project-rules/tasks.py`: boards of
drc_failure_boards.csv whose only new findings are clearance,
track_width, copper_edge_clearance or hole_to_hole, with a licence;
the expansion tier's machinery, 30 s, `--jobs 8 --threads 1`, no row
cut). x1274 is main at b306347 (identical to x1244 on the expansion
tier), x1275 this branch rebased on it. New findings (KiCad's errors
beyond the designer's; the strict column matches them by place):

| board | rule behind the findings | x1274 new / strict | unconn | vias | x1275 new / strict | unconn | vias | pass |
| --- | --- | --- | ---: | ---: | --- | ---: | ---: | --- |
| dacha compute r4 | regex class pattern (HIGH_V) | clearance 35 / 37 | 0 | 66 | 0 / 0 | 0 | 73 | no -> yes |
| dacha compute r5 | regex class pattern (HIGH_V) | clearance 35 / 37 | 0 | 68 | 0 / 0 | 0 | 62 | no -> yes |
| flatmaxxing valid_board | KiCad's 0.2 minimum | track_width 125 | 0 | 5 | 0 | 0 | 5 | no -> yes |
| kicad issue25101 | KiCad's 0.2 minimum | track_width 199 | 0 | 20 | 0 | 0 | 20 | no -> yes |
| EEGFrontier | KiCad's 0.2 minimum | track_width 166 | 0 | 21 | 0 | 0 | 20 | no -> yes |
| emonWiFi | .kicad_dru edge 0.3 | edge 10 | 0 | 5 | 0 | 0 | 10 | no -> yes |
| kisurf issue24211 | .kicad_dru on component classes, pair rule | clearance 11 / 17 | 0 | 2 | same | 0 | 2 | unmodelled, named |
| ChengminLi flyback | .kicad_dru on courtyards | clearance 1 | 0 | 10 | same | 0 | 10 | unmodelled, named |
| AQEYE, name-card, meshsat | net class (KiCad's fast path) | clearance 2, 2, 6 | 0, 0, 2 | | same | | | geometry |
| michpro, buganini, facade60 main | board setup edge | edge 1, 4, 3 | 0 | | same | | | Margin graphic, NPTH pads |
| nuclone (Squantor) | board setup hole to hole 0.5 | hole_to_hole 2 | 0 | 21 | same | 0 | 21 | same-net via to pad hole |
| 10VBlocker, HP303B, sonde, pic_programmer, TFRPM01 | (passing both) | 0 | 0 | | 0 | 0 | | yes, identical |

5/20 pass to 11/20 (strict 5 to 11), new errors 602 to 32 (strict 612
to 38), unconnected 2 to 2 (meshsat's both times), vias 489 to 494. The
rows left are not rules the router reads wrongly: kisurf's and
ChengminLi's rules are unmodelled and named; AQEYE, name-card and
meshsat are net class clearances KiCad finds closer than the router's
check (KiCad's fast path names no rule); the edge findings pair tracks
with "Segment on Margin" or "NPTH pad of J1" (KiCad keeps the edge
clearance from Margin graphics and NPTH pads, the router does not:
lowering, for the ladder agent); nuclone's is a via 0.464 mm from a pad
hole of its own net (hole to hole holds within a net in KiCad).

**Expansion tier** (x1275 against build/expansion-x1244.log, 8 x 1, no
row cut): 14/46 pass both (strict 14 both), unconnected 807 to 787, new
errors 291 to 20 (strict 17), vias 4038 to 4022. Five rows changed, all
on projects whose rules changed; the 41 others are identical (routed,
vias, length, KiCad's findings, work):

| board | what changed in its rules | x1244 | x1275 |
| --- | --- | --- | --- |
| tuxkeychain | Power class without clearance takes Default's 0.1 (was 0.2); .kicad_dru trace spacing 0.127 on every net, hole clearance 0.33; "Pad to Track" 0.2 unmodelled | 29/42, 23 unconnected, clearance 43 | 37/42, 5 unconnected, clearance 15 (9 from the unmodelled rule) |
| anthracite-bottom | GNDPWR matches both power and GND: GND's 0.2 by priority (was power's 0.5) | 137/139, 2 unconnected, starved_thermal 1 | 139/139, 0 unconnected, starved_thermal 4 |
| power-manifold backplane | .kicad_dru track width 0.15 floors the neck (was 0.127) | track_width 199 | 0 |
| AMLing2 drone carrier | .kicad_dru via to via 0.5 between nets | hole_to_hole 8 | 0 (10 unconnected both) |
| FUEL4EP CO2 | .kicad_dru PTH to track 0.33, NPTH 0.254, edge 0.3 | hole_clearance 38, starved_thermal 1 | 0 |

`experiments/project-rules/changes.py` predicts these from the files
alone (tuxkeychain, anthracite and the .kicad_dru boards).

**Short tier** (x1275 against x1274, 6 x 4, tasks v6): all 12 rows
identical (routed, vias, length, KiCad's findings, work); no short-tier
project has a .kicad_dru rule, a regular-expression pattern, a class
without a value or a minimum left to KiCad's default. Against
build/short-x1170.log the rows differ only as x1274's do (main's
thermals: vias 1134 to 1125, unconnected 19 both, 7 passes both).

Unit tests (project_rules and project_rules::dru): a file with comments,
units and escapes; a class clearance (case-insensitive); a rule on every
pair; a rule between two classes (unmodelled) and a class against every
other (applied); net names with wildcards and net identity; item types
and layers (pad to track unmodelled, track to track kept, an inner layer
on two layers skipped); conditions on courtyards, creepage and lengths
reported; widths, vias, holes, edge and hole-to-hole (with KiCad's
warning default and `hole_near_hole`); rules the classes keep; not
routing rules; three-valued, commutative evaluation; KiCad's defaults
for a project without minimums; regular-expression patterns and
composite classes; a project end to end (report, statuses, unmodelled).
`bash scripts/check.sh` and the clippy gate pass.

Logs: build/rules-x1225.log, -x1270, -x1271, -x1292, -x1273, -x1274,
-x1275, -x1276 (the commit: all 20 rows as x1275); build/expansion-x1271, -x1273,
-x1274, -x1275; build/short-x1274,
-x1275.

## Committed on 2026-10-21 (runtime calibration)

The review's finding: work budgets were seconds of one machine.
`EXPANSIONS_PER_SECOND` (3.15e6) and the placer's `WORK_PER_SECOND`
(150e6) were constants, the layout's wall guard the const 1.1, so any
machine slower than a 5950X core by more than a tenth (the macOS
snapshot's, CI runners, laptops, this one above load 45) had every
non-trivial layout cut (exit 3). Snapshots x1300 (b306347 plus this
change: the judges) and x1301 (rebased on the research removal). Logs and scripts in `build/calib2` (main tree).

**The rates are measured at the start** (`work_rate.rs`; docs/router.md,
"The machine's rates"). `route-kicad-board`, `layout-kicad-board` and
`place-kicad-board` first run a fixed search on a canned lattice (40 x 30
mm, two layers at 0.1 mm, 24 crossing nets, keepouts: 240 k nodes, about
50 MB of maps, about 600 k expansions a round) on one thread, five
rounds, and a fixed placement (40 parts, 20 global iterations, 15 M
units) as three copies at once on the process's threads (the placement
race's first batch), three rounds: 1 to 2 s in all, once per process,
before the run's clock. The fastest round's rate over the same workload's
on the reference (6.9 M expansions and 155 M units a second, measured
here at load 12 with `calibrate-work-rate`) scales the reference's
rates, now `REFERENCE_EXPANSIONS_PER_SECOND` and
`REFERENCE_WORK_PER_SECOND`; `expansions_per_second()` and
`work_per_second()` (set once, an atomic each) replace the constants at
every use. The pour analyses' nanoseconds and KiCad's check model scale
with the search's rate (`reference_time_scale`, exactly 1 at the
reference's rate). The placer's rate takes three copies at once because
the race's clock counts its seeds as the reference's eight threads run
them: on one CPU 0xcb-static's three seeds took 44 s of wall for 16 s of
counted work. The work counters are unchanged: a run is byte-identical
for given rates. Pinning: `PCB_WORK_RATE=<expansions/s>[,<placer
units/s>]` (a search rate alone scales the placer's alike; a bad value is
exit 2), else the router settings `work_rate`, `placer_work_rate`. The
first line of a run is `work rate: ...`; the verdict has `work_rate`,
`placer_work_rate`, `work_rate_source` (`measured`, `PCB_WORK_RATE`,
`config`, `reference`), `work_rate_measured`,
`placer_work_rate_measured` and `work_wall_ratio` (work over wall; with
several search threads the summed expansions can exceed the wall clock);
`PCB_WORK_RATE` joins the echoed environment; `layout: done` prints the
ratio and the rates. `pcb-maker calibrate-work-rate [rounds]` prints the
measurement alone.

**The wall guard** is the layout config's `wall_guard` (default 2, was the
const 1.1; below 1 is an input error): with measured rates it is a load
margin, not a budget. run.py's timeout without `--timeout`: the budget
times the guard plus 300 s, at least 1800 s (the short tier 800 to 1800 s,
the full 1800 to 3300 s). run.py pins the reference's rates for every
board (`--work-rate`, default `3.15e6,150e6`; `measured` lets each binary
measure), copies the verdict's rate fields into the rows, and
summary.json has `work_rate` and the medians of the measured rates and
of work over wall. The fingerprint and walkthrough tests pin the rates.

**Spread of the measurement.** The canned lattice is beyond the caches, so
the rate follows other jobs' memory traffic and bursts on the core or its
sibling: unpinned at load 12 to 13, 0.976, 0.988 and 0.998 of the
reference's search rate and 1.01 to 1.06 of the placer's; pinned to one
core at load 16 to 18, 3.3 to 5.7 M canned expansions a second on eight
cores (0.48 to 0.83), and on one core one process after another 5.4, 5.0,
5.8 and 2.96 M; during the short tier's 6 x 4 (load 7 to 14 plus the
tier) the binaries measured 0.78 to 0.96 of the reference (median 2.77 M,
placer 129 M). The fastest of the rounds counts, not the median, for
that reason; a run that must be repeated pins its rates.

**Judge 1, the review's experiment** (0xcb-static, total_seconds 40,
move 16, polish 2, `taskset -c 0`, the quick tier's prepared board).
x1292: exit 3, cut (deadline, placement, layout), 20 s of work at 50 s
of wall: the placement took 44 s of wall for 16 s of work, the race, the
moves and the neck repair were skipped, KiCad 32 unconnected. x1300, two
runs at load 12 to 13: not cut, exit 1 (KiCad 0 unconnected, the
designer's solder mask bridges), measured 2.88 M (0.91 of the
reference; placer 0.35 on one CPU) work 32 s at 32 s of wall (1.00), and
measured 1.60 M (0.51: a burst during the measurement) work 47 s at 31 s
(1.52; the check charge scales too, 7 s instead of 3.6). Two runs of the
development binary at load 20: work 48 s at 40 and 34 s of wall, not cut.

**Judge 2, the rate pinned at the reference** (short tier, `--jobs 6
--threads 4` under the lock, x1300 against x1244, the coordinator's
before side `build/short-x1244q.log`): every row identical apart from
timing and the new fields (same 12, unconnected 19, passes 7, strict 7,
vias 1125, 21142 mm, work 2517 s), every result board byte-identical
(md5). Against `build/short-x1170.log`: same 12, unconnected 19, passes
7; vias 1134 to 1125 and 21228 to 21142 mm from the repairs since. Wall
356 s, no row cut.

**Judge 3, three short-tier boards at the measured rate** (`--work-rate
measured --jobs 3 --threads 4` under the lock, x1300, load 8 to 9): the
binaries measured 3.12, 3.06 and 2.94 M expansions a second (0.989,
0.970, 0.934 of the reference) and 151, 147, 138 M placer units (1.006,
0.981, 0.923). urchin, spell_tome and USB_Cable_Tester routed as with the
pinned rate (68/68, 18/18, 54/54; the same vias and length, all pass and
pass strict), work 40.9 / 31.0 / 132.8 s against 40.5 / 30.1 / 124.0 s
pinned, wall 31 / 41 / 87 s. At load 20 the measurement is far lower:
unpinned 2.84 to 2.86 M canned (0.41 of the reference; 4.55 M, 0.66, a
moment later), pinned to CPU 0 0.47 to 0.52, other cores 0.48 to 0.83.
The canned search's best rounds below load 10 (the median binary,
`build/calib2/ref/idle.out`): 7.1 to 7.2 M pinned (1.03 to 1.04 of the
reference constant), unpinned 5.6 to 7.2 M.

Unit tests: the rate's parsing and its precedence (variable, config,
measurement, reference), a search rate alone scaling the placer's, the
canned workloads' work fixed, the log line; the guard's arithmetic and
its config default; CLI tests of a measured, a pinned and a configured
rate and a bad one (exit 2); run.py's pinning, timeout and row fields.
`bash scripts/check.sh` passes.

Outside this package's files (coordination): the rename of the constant
at its uses in pcb-router (driver.rs, fixed.rs, negotiate.rs, profile.rs,
vias.rs, tests.rs: `expansions_per_second()`, and the pour work's scale in
driver.rs's `pour_work_seconds`), two lines in pcb-kicad's lib.rs (the
module and its exports), two router settings in board_router/config.rs,
route.rs's uses, and run.py (the pinning, the timeout, `verdict_extras`
and the summary).

## Committed on 2026-10-21 (quality: pcb-kicad research removed, lib.rs split)

The last package of the quality pass: pcb-kicad's research code removed
and what is left of lib.rs split into modules, with placements and routes
unchanged. Branch `quality-kicad-research`, five commits on b306347
(written on 0d13e80, rebased four times); snapshots x1250 (the four steps
on 0d13e80), x1251 (on 1819890), x1252 (on 9da1213), x1253 (the five on
6868f9a), x1254 (on b306347). **The last commit holding the removed code
is b306347**; check it out to run or read any of it. Rust goes from
120 258 to 82 959 lines (on b306347).

**How the reachability was found.** pcb-kicad built as a binary rooted at
lib.rs, with src/main.rs, agent_cli.rs and freerouting_backend.rs as its
modules and every `pub` made `pub(crate)`: rustc's dead-code pass then
reports every item no command reaches (board_layout, board_router,
board_placer, verdict and the agent commands included, since main reaches
them). The removal was driven by that report and the compiler's errors
(impls of removed types, use lists), every step checked by the build.

| Step | What | Rust lines |
| --- | --- | ---: |
| 1 | nine research commands of src/main.rs and the island-bridge stage of route-kicad-board-freerouting; eight experiment scripts that only drove them | -469 (Python -1 012) |
| 2 | pcb-kicad's research modules and the research half of lib.rs (17 367 to 4 652 lines), their tests | -29 270 |
| 3 | the crates layout-trace-model, pcb-core, pcb-engine, pcb-grid-router; four benchmarks/imported fixtures | -7 731 |
| 4 | lib.rs split into eleven modules as moves (lib.rs 145 lines) | +165 (headers, rustfmt reflows) |
| 5 | pcb-kicad clippy-clean for the CI gate | +6 |

**Commands removed** (9): route-kicad-board-adaptive,
route-kicad-board-sequential, resume-kicad-board-sequential,
discover-kicad-vias, refine-kicad-vias (only experiments/whole-board drove
them), inspect-kicad-routing-cut, inspect-kicad-terminal-access,
inspect-kicad-route-obstructions (they inspect the research grid router)
and bridge-kicad-open-connections with route-kicad-board-freerouting's
optional `island_bridges` stage: no benchmark configuration sets it
(benchmarks/corpus/freerouting.json does not, and
benchmarks/github/freerouting.py runs the Freerouting jar itself, never
this command), and keeping it would have kept the research grid router it
routed with. The scripts removed with them: completion_stop_controls,
lazy_forecast_controls, route_placement_case, route_one_sweep (and its
test), routing_interference_order, run_via_discovery,
inspect_terminal_access and, in step 2 with island_bridge.rs that embedded
it, inspect_native_islands (and its test). area_probe.py still names
route-kicad-board-adaptive but already depended on `place`, removed on
2026-10-20; inspect_routing_cut.py stays because audit_partial_ripup.py
(a helper the Freerouting backend bundles) imports its renderer.

**Commands kept**: the agent commands; verify-kicad-rung (the
verification path board_router calls, `verify_materialized_rung`),
normalize-kicad-footprint-links (benchmarks/real/*/prepare.sh),
shrink-kicad-board (benchmarks/corpus/run.py), inspect-kicad-board
(pcbench/manifest.py, tscircuit, the Freerouting helpers) and
inspect-kicad-connections (small: board_stats), render-kicad-layers
(small: layer_render), score-congestion, score-congestion-sample,
export-congestion-sample (`congestion_samples::race_core_config` is the
race's), dump-pour-map, repair-kicad-necks, bench-search and
route-kicad-board-freerouting (benchmarks/corpus/run.py) with its external
stage and final-selection.json (a finished, preserved import with only
native opens is still the selection).

**Removed in pcb-kicad** (step 2): the modules connection_insertion,
semantic_template, sequential_router, joint_relaxation, adaptive_routing,
compound_recovery, sequential_resume, sequential_order_search,
connection_order, restoration_recovery, partial_ripup, island_bridge,
pad_pair, via_discovery, coupled_vias, routing_cut, terminal_access,
route_obstructions, route_failure, routing_demand (with its dense
reference), routing_grid, routing_cost, grid_alignment,
pad_layer_contacts and the route_union and rule_area test modules; in
lib.rs the research routing model (KiCadGridRouteConfig,
KiCadRoutingModel, the obstacle grid, grid-path materialisation), via
actions, route relaxation and shortening, reference fields, route
candidates and their import, ground zones, yielding diagnosis, the
semantic ladder's materialisation and the route-only verification. Kept:
`KiCadConnectionRoutingRules` (its resolve, validate and apply went),
`pad_clearance`, roundrect_pad, electrical_terminals' `plated_pad` and
`contact_layers` (layer_render), shrink.rs, `verify_materialized_rung`.
Beyond deletions: the route-graph normalisation no longer counts contacts
and inserted points (only research read them) and `LoweredPad` loses its
unread `layers`. pcb-kicad loses 157 tests (367 and one ignored to 210
on 0d13e80; 225 remain on 9da1213), all of removed code. route_obstructions' roundrect tests
went with it: they test its obstruction report, and roundrect_pad.rs's
own tests cover the corner gap and the rejected shapes. The pad and outline
geometry tests stay, on a test-only `geometry_queries` module (the
`contains` and `intersects_segment` queries the research model used, as
their oracle).

**lib.rs split** (step 4): kicad_sexpr (383 lines: Expr, parser, writer,
forms), kicad_nets (292), copper_emit (241), geometry (67),
copper_geometry (808), route_graph (289), board_stats (1 014),
verification (678), fs_util (90), layer_render (472), footprint_links
(336); lib.rs keeps the module list and re-exports (145, a crate doc). A
line multiset check of old against new finds only the module headers,
`use` lines and two rustfmt reflows. Each module takes the crate's names
with `use super::*` and lib.rs each module's with a glob, so paths inside
the crate stay; items, fields and methods another module uses are
`pub(crate)`, the rest private (all made private, then the ones the
compiler asked for restored). Two line changes:
`kicad_verifier_implementation_sha256` also hashes kicad_sexpr.rs,
kicad_nets.rs, verification.rs and fs_util.rs (lib.rs held them, and the
verification cache must still invalidate when they change: its keys change
once), and native_report's tests name `crate::verification::`.

**Clippy** (step 5): what the removal left of the CI gate's lints
(`cargo clippy --release -p pcb-router -p pcb-kicad -p pcb-placer -- -D
warnings`, 20 lints at 6868f9a, 9 of them in removed code) fixed in the
production code: let chains for two collapsible ifs (native_report),
`as_chunks` (congestion_rank), a function for a closure (labels), `if let`
for a single match and `split_whitespace` without `trim_start`
(netlist_import), type aliases `PinNet`, `NetChange` and `Choice`
(netlist_import, pin_swap), the positive-size check by `partial_cmp` (NaN
still rejected, net_classes), an iterator in shrink; with `--all-targets`
also two test asserts. The gate passes, pcb-kicad with `--all-targets`
too.

Not built any more: experiments/engine-playback (a standalone crate on
pcb-core and pcb-engine), like experiments/placement-exploration since
2026-10-20.

**Gates.** After every step `scripts/check.sh` (the library tests, the CLI
exit-code, walkthrough and native-verification suites, the Python tests)
and `cargo clippy --release --workspace`: no new warnings (22 at 0d13e80,
13 after step 2, the 9 gone in removed code). place-kicad-board on the
short tier's prepared x1170 sources (each board's layout.json, 4 threads,
`PCB_PLACER_WORK_SECONDS=120`): spell_tome_bottom, USB_Cable_Tester,
urchin, eternal-keypad, OpenFC and Blue_Line byte-identical between x1225
(main's) and x1250 (`build/qe-place`, `build/qe-place2`).
Short tier (tasks v6, 6 jobs x 4 threads, under the lock): x1250 (the
four steps on 0d13e80) and x1251 (rebased on 1819890) against
build/short-x1170.log: 11 of 12 rows identical but CPU, wall, load and
memory, and MIDAS-MK1.1's work 285.1 to 287.5 s, which is main's MIDAS
change (x1223 has it); against main's own x1223 and x1261 every row is
identical, and every routed board byte-identical to x1230 (the writer's
footprint-graphics order of 3d28004 makes x1170's and x1223's files
differ from all later ones) and to x1261: 7/12, unconnected 19, new errors
2, vias 1134, length 21228 mm, work 2504 s, cut 0, first-route open
terminals 26. Schematic tier (x1250) against build/sch-x1161.log: the rows
differ in `work_seconds` and `kicad_check_work_seconds` only (x1161
predates the check work accounting), and TelemetryOnboard 108/108 to
107/108, as the pcb-kicad quality pass found (x1170 gives it too); against
x1194, that pass's last schematic run, every row identical but the DRC
line's report path and cache hit, every board the same lines in another
order (3d28004's footprint graphics); 9/12, unconnected 5, vias 3190.
Expansion tier (8 jobs x 1 thread): x1250 against
build/expansion-x1231.log: all 28 rows uncut in both identical (13 cut in
x1231, 18 in x1250 at that load). x1252 against build/expansion-x1261.log
(main's reference since the bounded route-mode budgets): no row cut in
either; routed, vias, length, KiCad's counts, work, open, outcome and pass
identical on all 46 rows and every routed board byte-identical; the rows
differ only in what main's 9da1213 and 6868f9a add (`verdict`
router_complete_unverified and exit 1 for the 13 complete rows, as the
tier skips KiCad in route mode; the strict and repair fields). x1253
(the five steps on 6868f9a), short tier: every routed board byte-identical
to x1251's, the same objectives. **On the final base** (b306347, main's
thermals package; x1254): the short tier against a run of main's x1244
(`build/short-x1244q.log`) and the expansion tier against
build/expansion-x1244.log, every row identical but CPU, wall, load and
memory and every routed board byte-identical (short: 7/12, unconnected 19,
new errors 2, vias 1125, length 21142 mm, work 2517 s; expansion: 15/46,
unconnected 807, vias 4038, work 1279 s, cut 0); the outcomes as in
build/short-x1170.log (7/12, unconnected 19), whose copper the thermals
package changed. scripts/check.sh, the clippy gate and rustfmt pass on
b306347.


## Committed on 2026-10-21 (starved thermals at scale)

starved_thermal was the second largest finding type the router adds
beyond the designer's board in the 5 834-board intake: 120 boards, 103
blocked by it alone (benchmarks/expansion/data/analysis/drc_failure_boards.csv).
Snapshots x1240 (first candidate), x1241 (the thermal changes, on
0d13e80), x1242 (island stubs off) and x1243 (both commits rebased on
9da1213).
Working files, scripts and logs: `build/thermal-scale` (`tools/`:
`static_census.py`, `rows2.py`, `findings.py`, `table.py`, `cmp.py`,
`mktasks.py`; `starved-tasks.json`).

**Census.** `census.py` on the intake's routed boards of all 103 (the
x1185 model on their final copper, KiCad's side refilled): 252 starved pad
rows, 69 predicted, 145 missed, 38 with no model row, 9 false alarms;
very unlike the 44-board census (162 / 7 / 2 / 26). The same census on
the stripped boards (no tracks) split the findings into pads KiCad gives
fewer than two spokes before anything is routed (the designer's own
tracks gave the second) and pads the signals took a spoke from. Per new
finding (by pad and layer, the designer's own findings at the same pad
left out), route mode on the 40 of the 103 with a usable licence, x1185:

| Cause | Findings | Empty board < 2 spokes | Fix |
| --- | ---: | ---: | --- |
| model sees no spoke at all (no finding to it), KiCad one | 22 | 13 | spokeless pads get a track |
| predicted, not repaired: the signals took the spoke (a pocket around the pad, a track across the strip) | 12 | 0 | `free_lost_spokes` |
| predicted, not repaired: manual spoke cut at a junction of the net's own track, or none straight | 7 | 7 | manual spokes verified, no own copper on the way |
| model optimistic: another net's zone of a higher priority (USSTRocketry, smkent), meeting spokes, ends against tracks | 14 | 7 | zone clearance in the fill probe (part) |
| model counts a track KiCad does not (segment cut at a junction, end in a margin) | 8 | 3 | as above (part) |
| no model row: an exposed pad set to solid made its thermal vias solid | 5 | 5 | solid applies to the terminal's pad |
| no model row: a small solid zone of the net made the layer solid | 2 | 0 | solid per containing zone |
| no model row: per-label isolated connection (74th's J3.S1) | 2 | 0 | lowering (not done, below) |
| census could not match the finding to a pad (pads of one number in several zones) | 13 | | |

85 findings (x1185; x1232 identical but for rows the wall guard cut).
After, on x1241's routes: 44, of them 11 predicted and not repaired, 11
model-zero (tag-designs' island-only U5.6/U5.7, robotique's buttons),
8 optimistic, 5 track miscounts, 2 J3.S1, 7 unmatched. The census on
the intake's routes with x1241's model: 99 predicted, 151 missed, 2 no
row, 18 false alarms (the newly modelled vias inside solid exposed pads on
the near side, OpenAir's U6.17 F.Cu: KiCad gives them four spokes, the
model one; a false alarm only costs a repair attempt).

**Changes** (crates/pcb-router; docs/router.md, "Starved thermals at
scale"):

1. `kicad_spokes`: a label's solid connection is its terminal pad's; pads
   merged into it keep thermal records and rings (`thermal_ring_pads`),
   and `add_inner_spokes` reaches them inside any pad of their terminal on
   the layer. A pad is solid where the zones containing it are, not where
   any zone of the net on the layer is.
2. `Board::excluded_clearance`, `FillProbe`: an excluded region that is
   another net's zone keeps the zones' clearance plus half the minimum
   width (waived at the net's track ends). Tried in the pour masks too and
   left out there: OpenFC (short tier, layout) lost two connections in its
   first route, robotique's ladder moved to the tracks rung.
3. `add_manual_spokes_of`: candidates sorted by length, each kept only
   when the model then counts it (the first four tried), none passing a
   node of the net's own copper.
4. `Config::spokeless_tracks` (on; `PCB_ROUTER_NO_SPOKELESS_TRACKS`):
   one track into kept fill for a pad the model sees without a spoke
   (`Router::spokeless`, `SpokeRecord::drawn`).
5. `free_lost_spokes` (`Config::lost_spoke_repair`, on;
   `PCB_ROUTER_NO_LOST_SPOKES`) after `free_thermal_spokes`; and
   `free_thermal_spokes` keeps a move only when the pour nets' starved
   spokes go down (Squantor's alubox: 96 vias at x1232 when its budget
   was left, 48).

Unit tests, one per cause:
`a_solid_pads_merged_vias_keep_their_thermal_spokes`,
`another_nets_zone_of_a_higher_priority_keeps_its_clearance`,
`a_pad_without_a_spoke_in_the_model_gets_a_track_into_the_fill`,
`a_net_crossing_a_starved_pads_spoke_is_routed_around_it`.

**Gates.** Both sides of every route-mode pair below (x1232 and x1241,
x1225 and x1242) predate main's work bound for route mode (9a2cf41).

- Census, 44 boards (fill3 KiCad cache): 162 / 7 / 26 / 2 for x1241 as
  for x1184, every verdict line the same (`build/thermal-scale/census/c44-x1241.txt`).
- Route mode, the 40 licensed boards of the 103 (`starved-tasks.json`,
  expansion task format, fetched by run.py, 8 x 1, 30 s), x1232 against
  x1241: new starved findings 69 to 36, boards with one 39 to 19, passing
  1 to 19, other new errors 0 to 0, KiCad unconnected 4 to 2 (Apex 3 to 1
  and OpenAir 1 to 0, mdiph 0 to 1), vias 1542 to 1532, length 47742 to
  47343 mm, work 958.6 to 985.4 s. The first 20 boards: starved 19 to 8,
  passing 1 to 12, unconnected 0 to 1 (mdiph), vias 692 to 653.
  Worse rows: embeddedos 3 to 4 and tag-designs fc8db 1 to 2 (a pad the
  model sees without a spoke and no straight track reaches, on a changed
  route); unchanged: ray4477's vias on In1.Cu (no pad holds them there).
- Expansion tier (8 x 1) x1241 against `build/expansion-x1211.log`:
  every one of the 31 rows neither run cut has KiCad's findings and
  routed count unchanged; 21 of them changed copper (manual and spokeless
  tracks, moved nets): vias 1141 to 1139, length 36424 to 36267 mm, work
  1366.1 to 1369.6 s. Passes 14 and 14; 15 rows cut (13 before; load
  about 23), their unconnected 456 to 508 is theirs.
- Short tier (tasks v6, 6 x 4 under the lock) x1241 against
  `build/short-x1170.log`: 12 same, none better or worse; unconnected 19
  and 19, new errors 2 and 2, passes 7 and 7, first-route open terminals
  26 and 26, vias 1134 to 1125, length 21228 to 21142 mm, work 2501 to
  2517 s, none cut. Four rows identical (USB_Cable_Tester, eternal-keypad,
  urchin, Blue_Line); MIDAS the same copper with 2.4 s more work (the
  spoke analyses); the other seven changed copper by the repairs, summed
  over each layout's routes: nets moved off lost spokes on 0xCB (4),
  USB_Keypad (5), WLED_Matrix (17), reCamera (20) and spell_tome (14);
  manual spokes kept after the check on CRImier (5 lines to 9), WLED (48
  to 31), reCamera (53 to 181); OpenFC's `free_thermal_spokes` kept 5
  moves where it made 49 (159 vias to 146). The candidate x1240 (the zone
  clearance in the pour masks too) lost OpenFC (82/82 to 80/82, its first
  route 1 open terminal to 2); x1241 leaves the masks alone.

**Island stubs off** (Florian's decision; snapshot x1242, x1241 with
`Config::island_stubs` false; `PCB_ROUTER_ISLAND_STUBS=1` turns them on,
`PCB_ROUTER_NO_ISLAND_STUBS` is gone; the island vias stay on). A stub
changes no connection, it only silences KiCad's isolated-island finding.
What it costs:

- Short tier: nothing. No layout of x1241's short tier added a stub in any
  attempt, so the switch reads the same code path.
- Full corpus: v17 (x1170) has 168 island-stub lines on 29 boards, 9 of
  them passing. Route mode on those 9 placements
  (`--route-from build/github-layout-v17`, tasks v7, 8 x 1,
  `PCB_ROUTER_NO_WALL_GUARD=1`): x1225 passes 9, x1242 8, x1242 with
  stubs on 9. The one lost: mackerel-30-proto (2 starved thermals beyond the
  designer's, four in KiCad's report, all "spokes connected to isolated
  island" of through-hole pads on In1.Cu and In2.Cu; x1225 added 3 stubs
  there). Island vias and the thermal fixes recover none of
  its two; they do carry the other eight, whose x1225 routes had stubs in
  some attempt (homebrew_486 3, Castor, k30, SNSP 1 each) and pass without.
  The 20 failing boards cannot lose a pass.
- Expansion tier: x1241 added a stub on 9 of its 46 rows (in some
  attempt). Those 9 again with x1242 (8 x 1 under the lock): the 7 rows
  neither run cut have KiCad's findings unchanged, LC_PAMV4b and flexypin
  still pass; ZSWatch (cut by the deadline in both runs, the same rung)
  loses its pass: its winning attempt's one stub, on X602.1, leaves "3
  spokes connected to isolated island" on F.Cu. Passes 14 to 13.
- Total: one pass lost in the short and expansion tiers (ZSWatch), one of
  the nine v17 passes (mackerel-30-proto); neither island vias nor this
  package's repairs recover them.
- The count reaches the result: `RoutingResult::island_stubs` (stubs
  this router's finishes added; pcb-topo fills 0), with a unit test. For
  pcb-kicad (verdict.rs, the board router's result): carry it from the
  winning attempt's `RoutingResult` into `KiCadBoardRouterResult` and
  board-router.json, read it in `KiCadRepairs::of` as
  `Some(result.island_stubs)`, and list `PCB_ROUTER_ISLAND_STUBS` in
  `RESULT_ENVIRONMENT` in place of `PCB_ROUTER_NO_ISLAND_STUBS`.

**Not done (pcb-kicad's lowering, board_router/).** The lowering lists
zone connections per pad label (`solid_pads`, `isolated_pads`); a label
whose SMD pads have `zone_connect 0` and whose through-hole pads inherit
the zone's (74th's J3.S1, a USB-C shield) makes all its pads isolated, so
the model has no row for the thermal ones. The fix: record the
connection per pad, as obstacle indices beside the labels (for example
`Board::isolated_pad_obstacles` / `solid_pad_obstacles`, filled where
`isolated_pads.push(label)` is now, from the pad's own `zone_connect`
else its footprint's), and let `kicad_spokes`, `thermal_ring_pads` and
`prepare_net` test the obstacle (the router side is a few lines).

## Committed on 2026-10-21 (harness validity)

An independent review found the benchmark harness reproducible but weak on
validity. Seven fixes on branch `harness-validity`, one commit each; no
Rust change but one test file.

1. **The lock is run.py's.** A tier run, or a run of a whole task list,
   takes `build/quick-tier.lock` itself (an exclusive flock, a blocking
   wait like flock(1)'s, so that it waits its turn beside the older
   `flock ... run.py` habit, which still works: a caller that holds the
   lock lets the run through) and prints who holds it (pid and command
   line from /proc/locks, again when the holder changes). `--no-lock`
   skips it. A first version polled with LOCK_NB and lost every release
   to flock(1)'s blocking waiters.
2. **The task lists are committed**: `benchmarks/agent-tasks/tasks/v6` and
   `v7` (170 KB each); every tier defaults to `tasks/v7` (v6 and v7 differ
   on six panels, none in the quick or short tier). Finding: tasks.json is
   the same file in github-tasks-v2 to v7 (sha256 c8f95caf5089), so
   `tasks_sha256` never told the lists apart; rows now also record
   `task_sha256` (the entry with its constraints' digest) and
   `board_sha256`, and compare.py marks a board INPUT when either differs.
3. **Objectives against the designer**: compare.py and run.py's last line
   give per-board medians (vias and length over the designer's, KiCad's
   unconnected items over the connections, the ground pours' cut by other
   tracks where rows carry scores) beside the raw sums;
   `designer-numbers.json` holds the designer's numbers (score-kicad-board,
   15 s for the 109 boards); `run.py --quality` writes quality.py's
   measures of both boards into each row (0.1 s a board), `compare.py
   --score` scores older runs. v16 against v17 (103 boards with the same
   input): vias x0.70 -> x0.72 of the designer's, length x1.03 -> x1.03,
   open 0.009 -> 0.009 of the connections (mean 0.164 -> 0.113).
4. **The designer tier** (`--tier designer`, `designer.json`): every
   corpus board routed on the designer's placement, stripped as the layout
   tier strips it, its own rules, 1500 s of work; `compare.py --gap`
   gives per board the placement gap (layout open less designer-route
   open) and the routing gap (designer-route open), with corpus totals:
   the deciding experiment for placement versus routing work
   (benchmarks/agent-tasks/README.md, "The designer tier"). Six boards
   proved the path (below); the full tier (about 5 hours) is not run yet.
5. **Licences** (benchmarks/github/licences.py, README "Licences"):
   harvest.py gates `fetch` as harvest_schematics.py does
   (`--allow-unlicensed` for private benchmarking), every board carries a
   licence class, and training data (`fetch --training`, the expansion
   harvest's `nn_license_eligible`) is permissive only. The corpus: 109
   boards, 42 without an SPDX licence (24 of them with a licence file
   granting one), 32 permissive, 12 weak-copyleft, 27 copyleft, 16
   unclassified, 4 restricted, 18 unlicensed (private benchmarking only;
   seven in the quick tier, MIDAS-MK1.1 in the short tier). The congestion
   model's training manifest (700 boards) is 205 permissive by SPDX.
6. **CI** (`.github/workflows/check.yml`): check.sh without the
   walkthrough, a lint job (`cargo fmt --all --check`, clean; `cargo clippy
   --release -p pcb-router -p pcb-kicad -p pcb-placer -- -D warnings`, red:
   20 lints left in pcb-kicad, outside the modules made clippy-clean on
   2026-10-20), the walkthrough where kicad-cli is installed (else a
   notice). `tests/router_fingerprint.rs`: route mode on the fence's
   2-layer board, 2 s of ladder work and 6 iterations, identical
   `PCB_ROUTER_FINGERPRINT` lines at 1 and 4 threads (about 15 s a run,
   no kicad-cli). act is not installed: the workflow was dry-parsed.
7. **`scripts/prune.sh`** lists (and with `--yes` removes) merged, idle,
   clean worktrees and snapshots no committed file names. Listed on
   2026-10-21, nothing removed: no worktree (every merged one was active
   within the week; the first listing ran `git status` in each and so
   refreshed their indexes), 102 snapshots (4 GB of 18).

**Checks.** `bash scripts/check.sh` passes (the fingerprint test included);
the Python tests of benchmarks/agent-tasks, benchmarks/github and
benchmarks/expansion pass (the expansion's
`test_timeout_kills_descendants_and_keeps_cpu_measurement` fails about
one run in three under load, on main as well). The short tier through the
new in-script lock and the committed v7 list
(`build/short-x1224-harness.log`, x1224, 4 x 2) against
`build/short-x1170.log` (v6): "12 boards in both: better 0, same 12,
worse 0", every row's routed count, KiCad findings, vias, length,
first-route open terminals and prepared constraints identical (work
2501 -> 2504 s, MIDAS-MK1.1's +2.4 s as on x1223). It waited 3037 s for the
lock behind the designer run and another agent's run, naming each.

**The designer tier on six boards** (`build/designer6-x1225.log`, x1225,
4 x 2, 2816 s wall), joined with v17 (x1170) by `compare.py --gap`:

| Board | Connections | Layout open (v17) | Designer-route open | Placement gap | Routing gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| ecad-viewer video | 371 | 201 | 0 | 201 | 0 |
| OpenESC-30x30 4in1 | 152 | 111 | 0 | 111 | 0 |
| nonSNES SNSP-CPU-01 | 363 | 136 | 27 | 109 | 27 |
| DSP-ADAU1452 | 190 | 68 | 3 | 65 | 3 |
| OnBoard woagboard | 91 | 8 | 1 | 7 | 1 |
| Business-Cards Batch_1 | 204 | 415 | 453 | -38 | 453 |

Totals: layout open 939, designer-route open 484; placement gap +455
(worse on 5 boards, better on 1), routing gap 484 (on 4 boards): 48 %
placement, 52 % routing, and the routing share is Batch_1 alone (453 of
484; KiCad counts 453 unconnected items on a panel of 204 connections, the
router 102 open). Without Batch_1 the five boards are 94 % placement gap
(493 of 524 open), so on these boards placement is the work; whether the
corpus agrees is the full tier's question (about 5 hours). No row was cut;
x1225 predates the ladder's work bound (4in1 used 2427 s of work against
the 1500 s budget), so the full tier should run on a binary with it.
Both comparisons are across binaries (x1170 layout, x1225 route): for
a decision, run the layout and the designer tier on one binary.


## Committed on 2026-10-21 (verdict integrity)

Three defects an independent review found in the verdict
(docs/agents.md, "Read the verdict"), fixed, and one measurement. Binary
x1290 (on 0d13e80).

**Exit 0 without KiCad's judgement.** `native_report.rs` deletes a
report before each check; a kicad-cli failure or timeout inside the
neck repair (`necks::repair_with`, its trial checks and its restore of
the unrepaired board) was swallowed into `necks.json`'s note and the
repair returned `kept: false`, so the router did not check again, the
result directory held no `drc.json`, and `route_facts` read that as
"KiCad did not check": the router's count decided, `complete`, exit 0
on a board the router calls complete, with the next action blaming
`PCB_SKIP_NATIVE`. Reproduced on WLED_Matrix's v17 placement in route
mode with a kicad-cli wrapper that fails every DRC after the repair's
`--save-board` (and `PCB_MAKER_NO_CACHE=1`: with the DRC cache on, the
restore's check of the unrepaired board is a hit and hides it): no
`drc.json`, `board-router.json` with `native` set, the verdict
`kicad_unconnected_items: null`, "(router's count; KiCad did not
check)" (exit 1 there only because the router left 5 connections open).
Fixed three ways:

- The neck repair keeps the result directory's board, `drc.json`,
  `erc.json`, `verification.json` and preview before it starts and puts
  them back when a check fails (`necks.json`'s note says so; `kept`
  false, `tracks` 0): the directory always holds the report of the board
  it holds. Same reproduction: `drc.json` there, the verdict KiCad's
  (12 unconnected items, `kicad_checked: true`).
- `route_facts` returns an error when KiCad checked the board (`native`
  set) but `drc.json` is missing or unreadable: an internal error, exit
  4, with the reason, never a verdict on the router's count.
- Without KiCad's check (`PCB_SKIP_NATIVE`, `skip_native_verification`)
  a route is never `complete`: the new status
  `router_complete_unverified` (exit 1) when the router's count finds
  nothing open, and `kicad_checked: false` in the verdict.

**The pass rule's baseline.** The verdict (`split_findings`) and run.py
(`native`) subtracted the reference's error count per type anywhere on
the board, so a new error of a type the designer also has was hidden,
and nine v17 rows had designer counts at KiCad's cap (199, 499 for
clearance), which are floors. New, the strict rule
(`split_findings_strict`, run.py's `strict_findings`): a finding is the
reference's when it has the same type and either the same items (every
uuid, the same set) still the same distances apart (a part's own
finding goes with the part: TelemetryOnboard's 72 matches are U1, U2,
J1 and J2's own pad clearances and mask bridges, placed elsewhere), or
items at the same positions within 0.05 mm; each reference finding
answers for one. A type the reference has 199 times or more is
`preexisting_unknown` and none of it is subtracted. The verdict decides
on `new_findings_strict` and keeps the old count as `new_findings`; it
gains `clean_without_subtraction`. run.py's baseline is KiCad's DRC of
the designer's board stripped as the task strips it
(`strip-kicad-tracks`: tracks, vias and small zones; the fixed geometry
on its own), through drc-kicad-board's cache and only for results with
errors that count; rows gain `new_findings_strict`,
`preexisting_strict`, `strict_unknown`, `clean_without_subtraction` and
`pass_strict` beside the unchanged `pass`; compare.py prints a strict
column beside each run's cell, the strict pass count beside the pass
count and the strict new errors in the objectives.

The v17 sweep re-judged (build/github-layout-v17, its result boards'
own `drc.json`, one baseline DRC per board): **46 of the 48 passes stay
passes**; 21 are clean without any subtraction, 25 pass with findings
the stripped designer board has at the same place. The two that do not:

| Board | Strict new findings | Why |
| --- | --- | --- |
| hackclub OnBoard krishveercard | clearance 199 (unknown) | The stripped designer board has clearance at the cap (199; 215 on the full board). Every one of our 199 is at a designer finding's place (J4's own pads at 0.17 mm against 0.2), so without the cap rule it would pass; but a capped count says nothing about the findings past the cap. |
| ASH-ART Qfwfq | starved_thermal 1, hole_clearance 199 (unknown) | C4 pad 2 (GND, B.Cu) has one spoke of two: the designer's three starved thermals are elsewhere, so it is our own. hole_clearance is capped on both boards (the switches' PTH against their NPTH pads, K33...); without the cap rule 155 of our 199 match and 44 do not, the two capped lists holding different subsets. |

**Repairs and environment.** The verdict and board-router.json
(board-layout.json) gain `repairs`: `neck_tracks` and `neck_track_mm`
(the connection-width repair's kept tracks; `NeckRepair` gains
`track_mm`), `pour_mode` and `pours_as_tracks` (the kept board's
`pours`), and `island_stubs`, `null`: the stubs are added in pcb-router
(`manual_spokes.rs`), which reports no count yet; counting them needs a
counter there (another package's). The verdict gains `environment`: the
result-changing variables that were set (`RESULT_ENVIRONMENT`:
PCB_ROUTER_SEED, PCB_ROUTER_SEEDS, PCB_ROUTER_NO_ISLAND_STUBS,
PCB_ROUTER_NO_WALL_GUARD, PCB_SKIP_NATIVE, PCB_MIN_CONNECTION_FLOOR,
PCB_KICAD_NO_NECK_REPAIR, PCB_KICAD_TIMEOUT, PCB_MAKER_KICAD_USER_LIBS,
PCB_PLACER_WORK_SECONDS, PCB_NARROW_RESERVE, PCB_PROBES_SIDE_BY_SIDE,
PCB_LADDER_STATS). run.py rows carry `repairs` (and `environment` when
set); compare.py sums the neck tracks, their length, the island stubs
where counted and the rows kept with pours as tracks. The router's
behaviour and the island stubs' default are unchanged.

Tests: verdict unit tests for the strict rule (the hidden same-type
finding, items moved together, the cap) and the unverified status;
run.py's strict rule in test_run.py; CLI tests for the three exit paths:
`a_route_kicad_did_not_check_is_never_complete` (exit 1, the
environment and repairs echoed), `a_kicad_failure_in_the_neck_repair_leaves_the_last_report`
(a fake kicad-cli that crashes after the repair's save; fails on the old
necks.rs) and `a_missing_report_after_kicads_check_is_an_internal_error`
(exit 4; the hook `PCB_MAKER_TEST_DROP_DRC` removes the report).

Short tier under the lock (x1290, 6 x 4, load 3-6) against
build/short-x1170.log (build/short-x1290.log): every routing row the same
(routed, KiCad's unconnected and errors, vias, length, first-route open
terminals; 299 s wall, cut 0); MIDAS-MK1.1's work 285.1 to 287.5 s, as
x1223 to x1261 have it since the placer commit 0d13e80. Passes 7 and 7,
strict 7: new errors 2 by the old rule, 202 strict (eternal-keypad's
hole_clearance at the cap, 199 unknown, and its shorting_items 1;
WLED_Matrix's connection_width 1, hidden before by the designer's
connection_width elsewhere, and its copper_edge_clearance 1). Repairs: 7
neck tracks (1.31 mm: WLED_Matrix 4, USB_Keypad 3), one board kept with
pours as tracks (WLED_Matrix). Binary of the rebased commit: x1291.

## GPU grid A* prototype (2026-10-21)

A bounded go/no-go experiment on branch `gpu-astar-prototype` (not for
main): the router's lattice A* as a wgpu compute kernel, against a CPU
reference that reproduces the router's search, on dumped negotiation
searches of real boards. Details, tables and how to run it:
[crates/pcb-gpu-proto/README.md](../crates/pcb-gpu-proto/README.md).
**No-go.**

- The dump (`PCB_GPU_DUMP`, `PCB_GPU_STATS`; a patch to pcb-router kept on
  the branch as its own commit): every negotiation search of iteration 1
  of `bench-search ... 3 auto` on the v16 placements of Castor, k30-SBC and
  jetson-nano, 1416 searches, with what each reads resolved for its net
  over the region it can touch, and the router's path and expansions.
- Determinism: the CPU reference equals the router on all 1416 searches
  (path node for node, expansions), and the GPU kernel (one search per
  workgroup, invocation 0 pops a four-way heap of the router's keys, eight
  plus layer-count invocations evaluate the steps at once) equals the
  reference on all of them at 1, 8, 64 and 256 searches a launch and in the
  negotiation's own batches, with and without guards against fused
  multiply-adds. Device: GTX 1650, driver 615.78.08, Vulkan, wgpu 30.0.1.
- Speed: one GPU search 0.18-0.21 M expansions a second (4.5-5.5
  microseconds an expansion: a chain of 17-22 dependent global reads at
  150-255 ns, measured by a pointer chase), 256 a launch 2.4-3.0 M (a launch
  lasts as long as its longest search), the negotiation's own batches
  0.18-0.22 M; the CPU reference 3.4-4.8 M on one thread, the router 3.0-3.6
  M (route mode, `profile rate`, x1184). Launch overhead about 0.05-0.07 ms.
  Windows are the whole board (corridors confine the search), so dense node
  arrays need 16-32 MB a search; the searches touch 0.4-1.8 MB (median).
- The share a GPU could take at identical results: the parallel batches,
  1-4 % of the wall clock in route mode (Castor 1.6 %, k30-SBC 0.8 %,
  jetson 3.9 %), about two nets each (2.1-2.3). A single search split with
  the router's order (speculative rounds of the next K pops, an ordered
  commit, cut where a pushed key undercuts) commits 10-17 expansions a round
  at K of 64-128 (`lookahead`): CPU parity on this card at best, behind an
  exact batched GPU heap.
- What would have to change first: the negotiation offering hundreds of
  independent searches per synchronisation at identical results (Jacobi and
  speculative batches were measured worse and removed on 2026-10-16; the
  parked corridor graph is the remaining candidate), or an exact batched
  device heap with a much lower dependent-read latency; and paged node
  storage. The README lists what a production backend would need besides.

## Committed on 2026-10-21 (ladder memory and work bound)

Two defects of the route-mode ladder that the 5 834-board intake exposed
(docs/benchmarks.md, "Expansion intake"): it sized its routers side by
side from /proc/meminfo alone, so a harness's cap went unseen (456 runs
killed at 3 GiB), and `ladder_budget_seconds` did not bound its work (279
runs cut by the 90 s wall guard on overrun work). Binaries x1260 (the
two fixes), x1261 (with the narrow-signal step's share, the commit).
Code in board_router/memory.rs, budget.rs, route.rs, attempt.rs;
docs/router.md, "Memory limits" and "The work bound in route mode".

**Memory limits.** The ladder reads the process's limits as it starts and
runs as many routers side by side (rung probes, rungs, seeds) as fit the
tightest: RLIMIT_AS (against the virtual size), the cgroup's memory.max
(v2, every level up to the root; v1's memory.limit_in_bytes), an
explicit limit (`PCB_MAKER_MEMORY_MB` or the new router setting
`memory_mb`, the smaller, against the resident size), and half of
MemAvailable as before. A router's peak is estimated from its lattice
maps (each attempt now reads its router's `memory: N MB in maps` and
reports it as `maps_mb`): 1.7 x maps + 460 MB resident, 2.0 x maps + 550
MB of address space, times 1.25. Before, a rung counted the process's
peak after the first attempt plus 2200 MB for a KiCad check that no
longer runs inside a rung, and a probe 1.5 times what building the first
router added to the resident size.

The ratio, measured with one router at a time (pours connect, no finer
pitch, no seeds), one thread, x1225: peak RSS over maps 1.05 (DevMubarak1
pidgin_translator, 244 MB of maps), 1.33 (sam17-power-v2, 739), 1.28 (nz,
298), 1.68 (WK-3, 763), 1.61 (PRODIGY-MOTHERBOARD, 580) on five intake
resource_limit boards; 2.54 (Castor, 264), 2.49 (k30-SBC, 319), 3.22
(jetson, 302) on the v16 placements. VmPeak over maps 1.41-1.98 and
3.19-3.80. The ratio falls with the maps (a router holds 200-600 MB
beside them: the board parsed and lowered, its pours; jetson's process
holds 285 MB before any router), so the estimate is a line through all
eight, not one ratio (which would size a 1 GB-maps router three times
too large). Decisions that change what the ladder attempts go by the
limit itself, a fixed number: no finer pitch whose one router does not
fit a limit of the process's own, no rung probes where two routers (the
leader stays while the next probe runs) do not. The log names the limits
(`memory limits: the address space limit (RLIMIT_AS) 3072 MB, 3033 MB
free; ...`) and every side-by-side count with the router's estimate and
the limit that held it; board-router.json has `memory_limits`.

The judge: ten resource_limit boards of the intake
(resource_and_cut_boards.csv), stripped as the intake did, routed at its settings (route mode, 30 s,
`seeds` 1, `skip_native_verification`, `deadline_seconds` 90, one
thread) under `prlimit --as=3221225472`, 3 at a time
(scratch driver: polls VmHWM and VmPeak, wait4's maxrss):

| board | x1225 RSS / VmPeak MB | x1225 | x1261 RSS / VmPeak MB | x1261 |
|---|---|---|---|---|
| embeddedalpha IMU_V100 | 2643 / 3037 | killed (allocation failure) | 1190 / 1549 | 7/19, 34 open |
| DevMubarak1 pidgin_translator | 2570 / 3034 | 6/7, 1 open | 763 / 960 | 6/7, 1 open |
| sssamuelll sam17-power-v2 | 2473 / 3068 | killed | 969 / 1268 | 31/33, 2 open |
| knewter kvm-carrier-fanout | 2802 / 3066 | killed | 1091 / 1191 | 0/51, 193 open |
| DownUnderCTF sirkit | 2724 / 3072 | killed | 1001 / 1192 | complete 104/104 |
| sundevilmotorsports CANSniffer | 2295 / 2626 | 11/57, 145 open | 1058 / 1331 | 11/57, 145 open |
| zOnlyKroks WK-3 | 2618 / 3062 | killed | 1317 / 1567 | complete 165/165 |
| ENACRobotique useless_box_2025 | 2794 / 3064 | killed | 720 / 894 | 0/9, 191 open |
| mark69163 PRODIGY-MOTHERBOARD | 2750 / 3072 | killed | 944 / 1268 | 162/257, 476 open |
| sikaxn nz | 2608 / 3072 | 29/65, 101 open, wall cut (64 s of work) | 825 / 999 | 28/65, 87 open |

Before, seven die and the three that finish come within 450 MB of the
cap (VmPeak 2626 to 3072 MB; DevMubarak1 ran its 0.05 mm rungs, 975 MB
of maps each, two side by side); after, all ten finish at 894-1567 MB of
address space, two of them complete. Four boards (sirkit, PRODIGY,
kvm-carrier, WK-3) run their rungs one after the other instead of
probing them (two routers do not fit 3 GiB), three skip a finer pitch
whose router does not fit (nz and useless_box at 0.05 mm, kvm-carrier at
0.075), each logged. The intake's own cap was a resident-size kill at 3
GiB (and RLIMIT_AS at 12 GiB): with
`PCB_MAKER_MEMORY_MB=3072` and no address-space limit the ten peak at
800-1393 MB resident. knewter's and ENACRobotique's boards route nothing
in either binary (no negotiation: their nets cannot be lowered to
anything the router reaches).

**The work bound.** In route mode (no layout `deadline_work`) the
ladder's whole work stays within `ladder_budget_seconds` and 5 %
(`LADDER_WORK_TOLERANCE`): every router has a work deadline (the first
attempt all that is left, the rungs of a pitch after it an even share of
what is left when they start, seeds a share each, the steps what is left);
negotiations at most half of what is left (the 60 s floor gave a 30 s
budget's rungs 60 s each); probes together at most half the budget
(3.75 s each on 30 s with four rungs, were 75 s); a finer pitch only if
its projected work fits; nothing starts with less than a twentieth of
the budget left. Where the narrow-signal step applies, the first
attempt's work is kept for it once that attempt leaves connections open
(x1261: x1260 left facade60's step no work, 20 open; with the share 7).
The layout's ladder is unchanged (every new rule is off with a work
deadline).

Expansion tier, 30 s, 8 jobs x 1 thread under the lock, x1231 (the
reference log) against x1261:

| | x1231 | x1261 |
|---|---|---|
| pass | 14/46 | 15/46 |
| rows cut by the wall guard | 13 | 0 |
| ladder work: median / mean / max | 52.7 / 65.4 / 253.8 s | 31.3 / 27.7 / 34.2 s |
| work, CPU, wall of the run | 3007 s, 4902 s, 337 s | 1276 s, 1553 s, 136 s |
| router's open connections (sum) | 216 | 293 |
| KiCad unconnected (sum) | 508 | 808 |
| outcomes | clean_slow 11, clean_medium 1, cut 9, at_budget 21, early 4 | clean_slow 4, clean_medium 9, at_budget 18, early 15 |

The ladder's work never exceeds 34.2 s (tuxkeychain; Wonkle 34.0: what
routers do past their deadlines, below). Every outcome change:

- Complete in both (12), now in less work and wall: bms (87 to 33 s of
  work), nixie_board, TokayEdge, ZSWatch (112 to 32), flexypin,
  magnetorquer, ICE40HX1K, musicbrain, mini48, yuiop29re, brickpico,
  Multi_saturation (10.5 s in both, identical): all pass; seven move from
  clean_slow to clean_medium (the class is wall time) and the four that
  were cut are not.
- Better: MakerTiles Screen 2 open to complete (its narrow-signal step,
  14 s), epdInky 13 to 3, Miniboard 5 to 2, nervous-ecg 2 to 1.
- Same open count (21): EMG_DAQ, Julkort, FUEL4EP, LC_PAMV4b and
  Barklind (both still pass), RS485M (its narrow-signal step, 0.6 s, now
  has its share: x1260 left it 2 open), BloopBox, noise, minuet, pdlc,
  rbo12864h, nrf24_remote, snaphat, dreamV1_0, 3117_AUSIGN,
  Zulassungskarte, CompassTag, atmega32u4, wanbeiyu, and the two that
  were cut (backplane, node_v6). Nine of them (and nervous-ecg and
  Miniboard above) move from incomplete_at_budget to incomplete_early:
  the class means less than 95 % of the budget in work, and they stop at
  18-28 s because the next attempt does not fit what is left (wanbeiyu:
  "skipping pitch [0.075]: projected 35 s exceeds the 13 s left").
- Worse, all boards that used to overrun (their work then in brackets):
  rpi_cm5_dronecarrier 3 to 8 open (129 s, cut), OpenRX-Mono 12 to 13
  (74 s), tuxkeychain 11 to 13 (201 s, cut), driver_cplex_rp2040 3 to 20
  (254 s, cut; KiCad 7 to 170 unconnected: its probes at 3.75 s rank
  the tracks rung first, at 75 s the connected pours), MSign 4 to 11 (138
  s, cut), OpenDeck 1 to 11 (46 s, cut), facade60 3 to 7 (58 s: its
  narrow-signal step had 19 s then, 9 s now), anthracite 1 to 2 (86 s,
  cut), Wonkle 90 to 136 (61 s, cut). In all 93 more open connections
  (router's count) and 317 more KiCad unconnected items, 263 of them
  driver_cplex's and Wonkle's.

At 300 s (`--budget-scale 10`, x1261, the nine boards that got worse
and node_v6, 8 jobs x 1 thread under the lock; work 289-329 s), against
x1231's 30 s rows with their overrun:

| board | x1231 (30 s, its work) | x1261 30 s | x1261 300 s | intake 300 s |
|---|---|---|---|---|
| rpi_cm5_dronecarrier | 3 open (129 s, cut) | 8 | 1 | 1 |
| OpenRX-Mono | 12 (74 s) | 13 | 12 | 12 |
| node_v6 | 21 (221 s, cut) | 21 | 20 | 21 |
| tuxkeychain | 11 (201 s, cut) | 13 | 8 | 10 |
| driver_cplex_rp2040 | 3 (254 s, cut) | 20 | 3 | 2 |
| MSign | 4 (138 s, cut) | 11 | 6 | 6 |
| OpenDeck | 1 (46 s, cut) | 11 | 1 | 1 |
| facade60 | 3 (58 s) | 7 | 5 | 5 |
| anthracite | 1 (86 s, cut) | 2 | 1 | 1 |
| Wonkle | 90 (61 s, cut) | 136 | complete, pass | complete |

The 300 s run recovers eight of the ten to x1231's count or better
(Wonkle complete, tuxkeychain 8, rpi_cm5 1); MSign and facade60 stay two
above it, at the intake's own 300 s result. Their x1231 rows had the
work the budget did not allow (MSign 138 s before the wall guard cut
it, facade60 a narrow-signal step of 19 s).

**The tolerance, measured.** The ladder plans within the budget and 5 %;
what a router does past its work deadline comes on top (a fresh router's
first negotiation iteration, the phases that finish a negotiation the
deadline stopped): at 30 s the ladder's work ends at 9-34.2 s
(tuxkeychain: four probes' first iterations and the continuation's
finish, 2.7 s past the bound), at 300 s at 289-329 s (MSign: its
narrow-signal step took 82 s with 72 left). So a budget B means at most
about 1.15 B of work on these boards, and the wall guard (3 B, at least
90 s) fires only where the machine runs the work at less than about 40 %
of the calibrated rate.

Short tier under the lock (6 jobs x 4 threads, x1261 against
build/short-x1170.log): every row identical apart from timing and the
tasks file's spelling, except MIDAS-MK1.1's work seconds (285.1 to 287.5,
also in x1223, x1240 and x1250: the MIDAS-MK1 commit); against
short-x1223 and short-x1250 every row identical, and all 12 result boards
byte-identical to x1230's and x1250's (against x1223's only the footprint
graphics' order, 3d28004's writer). CPU 3354 to 2930 s, 940 to 320 s of
wall, 0 cut.

Unit tests (board_router/tests.rs): the limit resolution (machine only;
the address space against the virtual size; every limit with the
tightest deciding, the cgroup level with the least free, the explicit
limit the smaller of setting and environment), cgroup paths of both
versions, the maps from a real router's memory report, and the bound's
rules (the bound, the probes' share, the negotiation, the rung shares,
the step's share and its time). `bash scripts/check.sh` passes.


## Committed on 2026-10-21 (parser and writer robustness)

The reader and writer defects that the 5 834-board intake
(benchmarks/expansion/data/analysis: model_error_boards.csv,
verification_unknown_boards.csv, source_issue_boards.csv and the four
boards with new malformed_courtyard findings in drc_failure_boards.csv)
put on us. Binaries x1230 (the code on f720ab7) and x1231 (rebased on
8877428, the pcb-router quality package). Each defect has a unit test
from the board's own geometry where its licence allows (MIT, BSD,
Apache), a synthetic equivalent otherwise. Each board was run as the
intake ran it (strip-kicad-tracks, route-kicad-board on the designer's
placement with the intake's router-config.json: 30 s of work, one seed,
90 s wall guard; then drc-kicad-board), with x1211 before and x1230
after (scratch runs, not kept).

| Defect | Boards | Cause | Fix | x1211 | x1230 |
| --- | --- | --- | --- | --- | --- |
| rule area "fewer than three points" | Boardoza LSM303, McbeEringi 30led, tobycm skibidi disc | a zone outline that is one arc ending where it starts: KiCad reads it as a full circle (its arc centre is halfway between start and mid; 30led's own B.Mask fill of that outline is a 26 mm circle), we as two points | `outline::pts_points` lowers a closed arc as the circle (`full_circle`, shared with the Edge.Cuts arc); so the areas are kept as KiCad keeps them, not skipped | error | routed 17/17, 26/26, 14/14; KiCad: no rule-area finding; Boardoza 80 track_width (0.127 mm tracks, see below), 30led hole_clearance 4 as the designer's, tobycm 5 unconnected (below) |
| custom-pad primitive gr_bbox | 2 kicad-assistant QA boards | the pad editor's number box, which KiCad parses as a proxy shape (`SetIsProxyItem`) and leaves out of the pad's copper | lowered to nothing | error | 86/87 and 62/63 at the budget, KiCad loads and checks them (2 and 3 unconnected) |
| degenerate copper polygon | BratwurstPower | an fp_poly of three points, the last the first, 2 nm wide, negative stroke: `pts_points` dropped the repeated point, two left | the closing point is dropped only while three stay: a zero-area shape, as KiCad keeps it (Edge.Cuts skips polygons with fewer than three distinct points as before) | error | 67/67, KiCad: 1 starved_thermal |
| roundrect pad without roundrect_rratio | jayis1 lumen-tap | required | KiCad's default 0.25 when absent; a ratio that is there must still be a number | error | 14/14, KiCad: 1 shorting_items + 1 solder_mask_bridge (the designer's: 2 + 2, 39 unconnected) |
| ambiguous net identity "/A15" vs "A15" | gametank exp_port_mod_board | a KiCad 10 board has no net table, and `note_net_names` read the bare names from the table alone: none, so "/A15" folded into "A15" | `note_net_names` also takes the pads' names; the writer spells a connection that is a bare net as it is (`written_net_name`; "/A15" then is connection "/A15", written "//A15", and `canonicalize_copper_net_names` finds each pad's spelling) | error | 41/41, KiCad: the designer's copper_edge_clearance 22 and invalid_outline 2, no shorts |
| image data on one 24 MB line | stonedDiscord EMP | the encoder writes a list's atoms on one line; KiCad reads lines of at most 1 MB | a `data` list one string a line, as KiCad writes it | KiCad cannot load the result | KiCad loads it: courtyards_overlap 9 (the designer's 9), 0 unconnected |
| footprint graphics reordered on write | AccordeurDeGuitare, 3 dacha boards | the canonical order sorts a footprint's graphics, and KiCad chains a courtyard in file order: MK1's courtyard (two zero-length lines, two arcs) closes in the designer's order only | `canonical_order` notes each footprint's source order of graphics (by a digest of its reference and graphics); `encode` writes them back in that order while they are unchanged (a flipped footprint or a moved label is written as held); keys and digests use `encode_canonical` | malformed_courtyard 3, 10, 22, 16 | 0, 6, 18, 14: the designer's counts |
| outline "do not form closed loops" although KiCad accepts it | 8 hermod keyboard halves, 4 panels (Pips801 minibadge, SEM-karifa rudolf, prrpls406 and programmer panels) | outlines drawn from 10-20 um segments: the chaining snaps each end to the nearest one seen before and drops pieces shorter than its 0.02 mm tolerance, which tears such outlines apart | when that chaining fails, KiCad's own (`chain_as_kicad`: integer nanometres, an unused exact hit first, else the nearest end within 0.02 mm, closed when the end is back within 0.02 mm of the start); outlines that chained before are untouched | strip-kicad-tracks fails | all 12 load and route: hermod 119/119 to 133/133 (one 123/124, cut), Pips801 99/99, rudolf 162/162 clean, programmer 16/16, prrpls406 27/162 at the budget (41 unconnected); KiCad's findings the designer's plus those below |
| "trailing input" | RoyalBlue54L-Feather | "(curved_edges no)filter_ratio 0.9)" in each teardrops form: KiCad's teardrop reader takes the "(" as optional, ours closed the pad and then the board | in a `teardrops` list a bare key starts a list that ends at its ")" | strip fails | loads; 43-46/68 at the budget (cut), 30 unconnected, the designer's other findings |

**Not fixed here** (in files other packages own):

- **SeanBP Edge-Card-CAEN, "no routable connections"**: its 70 nets are
  named "1", "3", ..., "139", and `board_router::routable_net` rejects
  names of digits only (meant for single-pin placeholders). Counting
  digit names shared by two pads as `note_jumper_nets` counts
  `unconnected-(` names (a two-line change, tried on a scratch build)
  routes 70/70, but KiCad then finds 70 unconnected: every net joins a
  test point on F.Cu and one on B.Cu 0.3 mm apart, which the router takes
  as one terminal; KiCad needs a via. Both for the board_router package.
- tobycm skibidi disc: copper rings drawn as `gr_circle` with a net on
  `(layers "F.Cu" "F.Mask")` are not modelled; KiCad: 5 unconnected.
- Boardoza LSM303: 80 GND tracks of 0.127 mm on a project of 0.2 mm
  minimum width (the board's own narrow width).
- hermod halves: hole_to_hole 62 and 14, hole_clearance 5 and 10 from
  vias near switch holes (a custom rule of 0.5 mm the router does not
  read); prrpls406: hole_to_hole 24.

**Tiers.** Short tier (x1230, `--jobs 6 --threads 4`, under the lock):
`build/short-x1230.log` equals `build/short-x1170.log` row for row
(routed, vias, length, work, KiCad findings): 7/12, unconnected 19, new
errors 2, work 2501 s. Expansion tier (x1231, `--jobs 8 --threads 1`):
`build/expansion-x1231.log` against `build/expansion-x1211.log`: 14/46
pass and the same outcome classes (clean_medium 1, clean_slow 11,
cut_wall_clock 9, incomplete_at_budget 21, incomplete_early 4); the 33
rows the 90 s wall guard cut in neither run are identical (routed, vias,
length, work seconds, KiCad findings, outcome); the 13 cut in both differ
as cut rows do with the load. Four stripped boards (EMG_DAQ_ADS1292,
Barklind, power-manifold backplane, anthracite-bottom) have another
SHA-256: their embedded files' `(data |...|)` is back on its lines, the
same tokens in the same order. None of the 46 boards is among the fixed
ones. x1231 and x1230 give byte-identical routed boards (four of the
boards above checked).

Tests: `outline::tests` (closing as KiCad, the closed-arc circle, the
degenerate polygon), `canonical_order::tests::footprint_graphics_are_written_in_the_files_order`,
`roundrect_pad::tests::roundrect_without_a_ratio_takes_kicads_quarter`,
`tests::a_custom_pads_number_box_is_no_copper`,
`tests::a_teardrop_setting_without_its_opening_parenthesis_reads_as_kicad_reads_it`,
`tests::embedded_data_is_written_one_string_a_line`,
`tests::colliding_normalized_pad_names_stay_two_nets_when_the_board_declares_both`
(now the KiCad 10 case); `route_obstructions`' malformed-roundrect test
no longer counts a missing ratio as malformed.

## Committed on 2026-10-21 (expansion tier)

The intake's proposed 46-board tier (benchmarks/expansion/analysis) as a
run.py tier: `--tier expansion` (benchmarks/agent-tasks/README.md, "The
expansion tier"). No Rust change; binary x1211.

- **Route-mode tasks** (`"mode": "route"` in a task): the designer's
  placement, fetched into `build/github-expansion/<task>/source` when not
  there (the harvest's copy when its SHA-256 matches, else GitHub at the
  pinned commit with harvest.py's fetch; every file verified, staged, and
  nothing kept on a mismatch), stripped with the intake's exact
  `strip-kicad-tracks <original> <copy>`, routed with `route-kicad-board`
  at the task's `budget_seconds` times the budget scale with the intake's
  router settings (one seed, no KiCad check inside), wall guard (max(90,
  3 x budget)) and process timeout (max(180, 3 x budget + 60)), then
  checked with `drc-kicad-board`. Rows add `license`, `fetched`,
  `fetched_from`, `source_sha256`, `stripped_sha256`, `budget_seconds`,
  `routing_seconds`, `open`, `drc_seconds` and `outcome` (the intake's
  class); `peak_rss_mb` covers the DRC too and the summary counts the
  outcomes. `constraints` is optional in every task.
- **Licences**: every task carries its repository's licence and every row
  prints it; a board without one (missing, empty, `NOASSERTION`) is not
  fetched.
- `expansion.json` (budget_scale 1: 30 s; `--budget-scale 10`: the deep
  run's 300 s) and `expansion-tasks.json`, written by `from_expansion.py`
  from the proposal, the CSV, the harvest's manifest and the quick run's
  results; `--harvest-data`; `--timeout` defaults to the intake's in route
  mode. compare.py reads the rows unchanged. Tests:
  `test_expansion.py` (task list, router settings, strip arguments against
  triage.py's, the fetch with a fake GitHub, the outcome classes).

**First run, x1211** (8 jobs x 1 thread, load 13-23, 471 s): 14/46 pass;
clean_slow 11, clean_medium 1, incomplete_at_budget 21, cut_wall_clock 9,
incomplete_early 4. 45 of 46 boards have the intake's outcome class (the
other is the same route in fewer wall seconds: B1's line is wall time).
All 46 stripped boards match the intake's, and all 32 routes that neither
run's 90 s wall guard cut are identical, work seconds included: the
1.57x calibration was already in the intake's binary (015676c precedes
be4b012) and the router has not changed since. 13 rows are cut by the
wall guard and depend on the load. Three B4 boards at 300 s pass with the
intake's deep work seconds.

## Committed on 2026-10-20 (MIDAS-MK1 refusal)

In layout sweep v17 (x1170, tasks v7) MIDAS-MK1 ended "incomplete: 6
parts without a legal place (J105, J101, J102, R204, R207, R208)" after
206 s; v16 (x862) had placed and routed it (151/159, 16 unconnected).
Snapshots x1221-x1223 are this work on 0425fea; x1224 is the commit on
3d28004 (MIDAS's poses as x1223's; the file differs only by 3d28004's
writer, which keeps footprint graphics in the file's order).

**Which rule.** None of the rules is wrong. Only J105 is unplaced; the
five others are fixed parts it was left on: J105 (a 7.7 x 35.4 mm Molex
terminal block held at the bottom edge, turned 90) stood over the back's
fixed R204/R207/R208 (its holes, "far side") and against the front's
fixed J101/J102. `PCB_PLACER_DESIGNER` calls the designer's J105, J103
and J108 legal (its 70 flagged parts are spacing at the default level).
The switches change nothing: `mask_openings`, `solid_pin_rows`,
`keep_copies` off give x1170's placement byte for byte,
`orientation_search` off another placement with the same refusal.
Bisecting the snapshots on `place-kicad-board` (the v17 source, three
seeds, 600 s of work, four threads): x862, x1112, x1130 and x1152 place
all; x1161 (order independence) refuses. The canonical order sent all
seeds' anneals elsewhere: J108 (a 24.7 x 19.1 mm SD slot held at the
left edge) into the bottom-left corner, where the designer has it at the
top left. Left of the fixed parts the bottom edge then holds one of the
two terminal blocks, not both.

**A search gap.** Largest first, the legaliser seats J108 in the corner,
J103 beside the fixed parts, and J105 finds nothing. With J105 first
(the retry) J105 takes J103's spot and J103 finds nothing; eviction moves
the parts on one spot but cannot reseat J103, which needs J108 moved.
J108 has a spot only once bodies may reach past the edge (the inset
levels: x862 put it 4.6 mm over the left edge, copper inside), and those
levels only keep the placement and try the stuck part among the kept
ones. With J105 and J103 both first, both stand at the bottom and J108
moves left over the edge by 1.9 mm.

**The fix** (`legal::legalize_stuck_first`, `lib.rs`'s kept-placement
path): at a level with no halo, spacing or grid left to give (the edge
and courtyard levels), when the kept placement leaves a few parts
without room after keeping and eviction (as many as an eviction takes
on), the level legalizes the anneal that gave the kept placement again
with those parts first; the parts that pass leaves without room join
them, up to four passes (`STUCK_FIRST_PASSES`), while new parts fail and
the work lasts; the pass with fewest parts left wins if it beats the
kept placement. MIDAS: J105 first leaves J103, J105 and J103 first leave
none. Two broader variants were measured and not kept: the passes also
in the fresh anneal's retry and at every kept level (x1221), or at every
kept level (x1222), seat stuck parts of other seeds at stricter levels
and change the seed kept on framework_mobo and OpenFC (16 seeds); x1222's
short tier lost framework_mobo's pass (70/71). A whole-board search on
the fine grid for parts placed first was tried too and not needed.
Unit test `parts_without_room_go_first_until_none_takes_anothers_place`
(tests/placement.rs): MIDAS's bottom edge simplified (the two blocks,
the slot in the corner, a fixed block on the right): `legalize` leaves
J105, `legalize_first` with J105 leaves J103, the passes leave none and
every part is legal.

**MIDAS-MK1** (layout, tasks v7, four threads, build/midas-x1223.log):
complete placement (3 seeds, wirelength 2295 mm against x1170's 2431 with
J105 unplaced and x862's 2628), 153/159, **6 unconnected** (v16 16), 0
new errors, 205 vias (v16 207), 4225 mm (v16 4565), work 1404 s, cut 0,
first-route open terminals 13 (v16 19). x1221's and x1222's routed boards
are the same file.

**Identity.** `place-kicad-board` on short-x1170's prepared sources
(each board's layout.json, four threads): spell_tome_bottom,
USB_Cable_Tester, urchin, eternal-keypad, OpenFC and Blue_Line
byte-identical with x1170; framework_mobo and OpenFC with 16 seeds too.
Short tier (jobs 6, threads 4, under the lock) x1223 against
build/short-x1170.log: 7/12 both, unconnected 19, new errors 2, vias
1134, length 21228 mm, cut 0, first-route 26 on both; every row
identical and every placed board byte-identical but MIDAS-MK1.1's work,
285.1 to 287.5 s: it is MIDAS-class (two terminal blocks held at the
bottom, J101 and J102), a seed's stuck parts took the passes at a
loosest level without a better result (build/short-x1223.log).

**The other v17 refusals** (`place-kicad-board`, v17 sources and
layout.json, 600 s of work, four threads; unplaced x1170 / x1223): the
molecule panels are the designer's own geometry (neighbouring tiles' J5
and J6 0.03 mm apart on the back, J9 against the outline; panel fixes,
2026-10-15), not this mechanism, and the passes seat some of them:
7x5_wave_left 1 / 1 and 5x2_shifted 2 / 2 byte-identical, 4x4 3 / 2,
4x5 7 / 3. OpenESC's 4in1-panel is crowding (111 / 111 unplaced,
byte-identical: too many stuck parts for the passes). SmartSpin2k
finished in v17 and was not rerun.

## Committed on 2026-10-20 (quality: research stack removed)

The research stack of August and September 2026 removed, as Florian
approved (branch quality-research: 346c9ba, 8ac4653, d988456, 9df9ee5 and
this section, on be4b012). **The last
commit holding all of it is be4b012** (main on 2026-10-16); check it out to
run or read any of the removed commands, crates, tests or fixtures. Rust
goes from 190 752 to 118 802 lines; 152 444 lines deleted in all.

| Step | What | Lines deleted |
| --- | --- | ---: |
| 1 | `crates/pcb-pipeline` (no `pcb_pipeline::` use anywhere) | 763 |
| 2 | 54 research command arms of `src/main.rs`, their helpers (the semantic ladder's placement and order portfolios, the resistor field, the config loaders), their unit tests and usage lines: `main.rs` 6610 to 1008 lines | 5 602 net |
| 3 | the crates `layout-trace-routing` (24 707), `pcb-routing` (12 862), `pcb-coordinator` (9 083), `pcb-placement` (8 750), `pcb-benchmark` (3 667), `pcb-validate` (2 637), `pcb-viewer` (532), with the root tests that only tested them (`exact_validation_smoke`, `occupied_passage`) | 62 567 |
| 4 | the root tests `kicad_ladder` (2 038), `kicad_pad_layer_contacts`, `board_continuation`, `conflict_actions`, `kicad_grid_alignment`, `kicad_via_keepout`, `kicad_pad_clearance`, `imported_benchmarks`; the fixtures `benchmarks/esp32-c3-ladder` (29 814), `dual-esp32-ladder` (12 060), `analysis-lab`, `small`, `esp32-pad-gaps`, `competitive`, `smoke` and `imported` but four files (23 790); the experiments `placement-locality`, `field-routing`, `occupied-passage` | 82 976 |

**Commands removed** (54): check, validate, insert-connection,
trace-placement, place, route-grid, route-grid-negotiated,
explore-conflict-actions, search-conflict-actions, continue-board,
place-route-grid, place-route-grid-width-continuation,
place-route-grid-negotiated, analyze-corridors, analyze-route-families,
materialize-route-family-portfolio, repair-route-families,
repair-route-grid, optimize-route-junctions, repair-route-pressure,
repair-route-order, repair-route-ripup, view-route, render-route-layers,
view-continuous-repair, relax-continuous, resistor-field,
resistor-ablation, generate-semantic-kicad-ladder,
solve-semantic-kicad-prefix, solve-semantic-kicad-order-portfolio,
solve-semantic-kicad-placement-portfolio,
solve-semantic-kicad-prefix-feedback, materialize-kicad-rung,
insert-kicad-connection, progress-kicad-connections,
relax-kicad-joint-routes, search-kicad-board-sequential-orders,
propose-kicad-pad-pair-route, route-kicad-connection,
diagnose-kicad-yielding-connections,
repair-kicad-single-connection-ripup, import-kicad-route-candidate,
apply-kicad-route-candidate, relax-kicad-reference-fields,
search-kicad-reference-fields, search-kicad-via-actions,
replace-kicad-connection-with-zones, measure-kicad-route-candidate,
shorten-kicad-route-candidate, relax-kicad-route-candidate,
benchmark-route-freerouting, benchmark-route-compare,
benchmark-route-corpus.

**Commands kept**: the agent commands (route-, layout-, place- and
drc-kicad-board; repair-kicad-silkscreen, strip-kicad-tracks and -copper,
import-kicad-netlist, export-kicad-fab, score-kicad-board,
describe-kicad-board, swap-kicad-pins), verify-kicad-rung,
normalize-kicad-footprint-links, shrink-kicad-board, inspect-kicad-board,
-connections, -routing-cut, -terminal-access and -route-obstructions,
score-congestion, score-congestion-sample, export-congestion-sample,
dump-pour-map, repair-kicad-necks, bench-search,
route-kicad-board-freerouting (benchmarks/corpus/run.py),
render-kicad-layers, and for the next package to decide
route-kicad-board-adaptive, route- and resume-kicad-board-sequential,
refine- and discover-kicad-vias, and **bridge-kicad-open-connections**:
route-kicad-board-freerouting's `island_bridges` option
([native-external-routing.md](native-external-routing.md)) runs it as a
subprocess.

**Kept for the pcb-kicad package** (they go with pcb-kicad's research
modules): the crates `layout-trace-model` (pcb-kicad's
`semantic_template.rs`), `pcb-core` and `pcb-engine` (pcb-kicad's
`lib.rs`), `pcb-grid-router` (`lib.rs` and `routing_cut.rs`, behind
inspect-kicad-routing-cut); no longer root dependencies. Four files of
`benchmarks/imported/layout-trace/` (dual-esp32-benchmark,
forced-crossing-two-layer, decoupling-capacitor,
esp32-slices/esp-pad-fanout-clearance) stay because the tests of
`semantic_template.rs` and of `layout-trace-model` include them;
`benchmarks/manifest.sha256` lists only those. `experiments/whole-board`
stays: `src/freerouting_backend.rs` and pcb-kicad's `island_bridge.rs`
include its scripts. `experiments/placement-exploration` (a standalone
crate on `pcb-placement`) no longer builds; it is history. Historical
documents (docs/experiments, docs/reviews, experiments/*.md) that name
removed commands or fixtures are left as written.

`benchmarks/README.md` is now an index of the benchmark directories;
`benchmarks/real/README.md` notes that its benchmark-route commands are
gone. `main.rs` (1 008 lines) needed no split.

**Gate**: `scripts/check.sh` passes after every step (455 library
tests, the three CLI suites, the Python tests); `cargo clippy --release
--workspace` has the same warnings in the kept crates as before (the
root's 2, pcb-kicad's 46, pcb-router's 26, pcb-topo's 11, pcb-placer's 6,
pcb-congestion's 2; pcb-placement's 12 left with it).

**Short tier** (x1210 = the branch's code at 9df9ee5, `--jobs 6 --threads
4` under the lock): `build/short-x1210.log` equals `build/short-x1170.log`
in every row but the CPU, wall and memory fields: 7 of 12 pass,
unconnected 19, new errors 2, vias 1134, length 21228 mm, work 2501 s, cut
0, first-route open terminals 26.

## Committed on 2026-10-16 (quality: pcb-kicad)

A code-quality pass over the layout, router and placer adapters of
`pcb-kicad` with no behaviour change. Binaries x1190 (step 1), x1191
(step 2), x1192 (step 3), x1193 (rebased on the research removal), x1194
(rebased on the pcb-router quality package, for the final gates), x1195
(this commit on 3d28004).

**Removed switches and options** (each restored code from before a change
the measurements settled):

- `PCB_LADDER_OLD_RESERVE`: the ladder's narrow-step reserve is a floor
  ("narrow reserve floor" below); `narrow_step_reserve` is the floored one.
- `PCB_LADDER_FORCE_RUNG`, `PCB_LADDER_SHORT_RESUME`: the ladder judge's
  experiment hooks (`experiments/congestion/ladder_judge.py` used them; the
  predictor is a closed negative). `PCB_LADDER_STATS` stays.
- `PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET` ("first-route budget").
- `PCB_LAYOUT_NARROW_CONTINUATION` and `_FLOOR`: after the narrow-signal
  attempt run alone only the other pour mode runs ("after the narrow
  attempt"); the stop, seeds and ladder variants and their seed helpers
  are gone.
- `PCB_KICAD_OLD_ZONE_ARCS` ("zone seams").
- Placer options `bus_rows` (with `bus_row_gap_scale` and the row
  constraint's internal `turns`), `supply_via_room` (with
  `supply_via_room_mm`) and `replicate_channels` (with
  `PCB_PLACER_CHANNEL_SELF`, the channel lowering and `lower_placement`'s
  channels argument): all off, none won. A placer config that still sets
  one is now rejected (the config denies unknown fields);
  docs/constraints.md still documents `bus_rows`.

- `jacobi_batch` in the router config and `PCB_JACOBI_BATCH` in
  src/main.rs: the router no longer reads it (pcb-router quality).

Kept: the user-facing kill switches (island stubs,
solid starved thermals, `PCB_KICAD_NO_NECK_REPAIR`) and every diagnostic
switch.

**Clippy** is clean on the files of this pass: dead code (the wall
guard's `Labels`, `Check` and `Report` steps, never asked: asking would
only fire the guard later; `courtyard_shapes`; `default_edge_rule`, a
serde default on a Serialize-only result; `turned_box`), the move trial's
restore before returning an error (the error ends the layout), type
aliases, allows with reasons for long argument lists and the negated
comparisons that reject NaN, `clamp` where the bounds are constants and
the value never NaN.

**Splits** (moves): `board_router.rs` (6750 lines) into
`board_router/{config, result, lower, tools, geometry, memory, budget,
route, pour_plan, attempt, tests}.rs`, `board_placer.rs` (5300 lines) into
`board_placer/{config, result, rules, bodies, decoupling, courtyards,
lower, edges, trace, seed_race, place, tests}.rs`; `mod.rs` re-exports, so
paths outside are unchanged. A line multiset check of old against new
finds only rustfmt reflows, the module lines and `lower_board`'s own copy
of `polygon_area` (the same products in the same order), which gives way
to the module's.

Lines: step 1 -1236 (148 added, 1384 removed), step 2 -39, step 3 +159
(module headers and rustfmt reflows of the moved code).

**Gates**, after each step: the tests of pcb-kicad, pcb-placer and
pcb-router and the CLI exit-code and walkthrough tests; place-kicad-board
on the short tier's prepared x1170 sources (spell_tome_bottom,
USB_Cable_Tester, urchin, eternal-keypad, OpenFC, Blue_Line; 4 threads,
120 s of placement work), boards byte-identical to x1170; layout mode, one
run a side, 4 threads, on TelemetryOnboard and MokyaLora (the ladder keeps
244 s for the step; Telemetry releases it complete) and vaio_re (the
narrow-signal attempt alone, then the other pour mode): rows identical
but CPU, wall and memory, boards byte-identical to x1170
(`build/qc-lay-x1170`, `-x1190`, `-x1191`, `-x1192`).
At the end, on x1194: the short tier under the lock (6 jobs, 4 threads)
against build/short-x1170.log, all 12 rows identical but CPU, wall,
load and memory, every board byte-identical (7/12, unconnected 19, new
errors 2, 1134 vias, work 2501 s); the schematic tier against x1193 (the
same code on the previous base) all 12 rows and boards identical. Against
build/sch-x1161.log the rows differ in `work_seconds` and
`kicad_check_work_seconds` only (x1161 predates the KiCad check work
accounting, d490aad), boards identical, but TelemetryOnboard (108/108 to
107/108): x1170, main's code before this pass, gives the x1193 row and
board exactly (build/sch-x1170-telemetry), so the change is the check
work's, not this pass's. On 3d28004 (x1195): scripts/check.sh, and
place-kicad-board on the six boards byte-identical to main's x1232.

Not taken (behaviour changes, for a later agent): `routable_net` rejects
net names made only of digits (SeanBP Edge-Card-CAEN); the writer's
footprint file order in a process-global registry rather than on the
board (this pass did not touch `encode`).

## Committed on 2026-10-16 (quality: pcb-router)

A code-quality package for `crates/pcb-router`, no algorithm change:
every step routes byte-identically to x1170 (main 54eff39). Branch
`quality-router`, four commits, snapshots x1180 to x1183 (one per step)
and x1184 (the four rebased on main, the gates' last binary). Finished on
2026-10-21 after a pause; the date is the package's start.

**1. Settled switches removed** (x1180; the router crate -350 lines).
Each restored the code before a change for A/B runs, and the changelog
section that introduced it records the measurement that settled it. Gone,
with their old code paths, the current path the only one:
`PCB_ROUTER_OLD_SPOKES`, `NO_MANUAL_SPOKES` (the fields `kicad_spokes`
and `manual_spokes` stay switchable: router.json sets them),
`NO_ISLAND_VIAS` (`island_vias`), `OLD_TRACKED` (`track_meets_spokes`),
`NO_FOREIGN_SPOKES`, `OLD_TRACK_PIECES` (`track_touch_fill`),
`NO_NECK_JOINS`, `OLD_SPOKE_FRAMES`, `NO_INNER_PADS`, `OLD_TRACK_ENDS`
(`track_end_tolerance`), `NO_INNER_SPOKES`, `OLD_TRACKS_FILL`
(`tracks_rung_fill` and `Router::hatch_any_plane`), `OLD_TRACK_RUNS`
(`track_end_junctions`), `NO_ZONE_SEAMS`, `OLD_SPOKE_CLEAR`
(`spoke_clear_to_fill`), `OLD_HOPELESS` (`unreachable_terminals`),
`OLD_RESOLVE` (`resolve_keeps_partial`), `OLD_FILL_CLEARANCE` (board.rs),
`OLD_POLYGON_DISTANCES` (grid.rs), `REROUTE_FRACTION`, and the
`jacobi_batch` option with `route_jacobi` (`PCB_JACOBI_BATCH` is still
read by src/main.rs into the pcb-kicad field, which no longer reaches the
router). `PCB_ROUTER_NO_ISLAND_STUBS` stays (a default for Florian). The
tests' "old" halves went; their current halves stay.

**2. Parked speculation removed** (x1181; -791 lines): `speculate`,
`speculate_cleanup`, `speculate_paths`, `route_batches` (now a loop over
`route_batch`), `ReadSet` and its notes in the search, the planner and the
per-search escapes loop (which only noted reads), `changed_cells`,
`mark_dirty_blocks`, `clean_up_window`, `clean_up_batch`, the speculative
part of `clean_up`, the profile line's speculation counts, and
`PCB_ROUTER_NO_SPECULATION`, `SPECULATE_PATHS`, `SPECULATION_CHECK`,
`SPECULATION_NO_TILES`, `CLEANUP_WINDOW`. The code is in e256657, which
added it (router.md, "Known limits"). Also `hard_thermal_guards` and
`stitch_in_negotiation` (off, measured worse in the 2026-10-09 sections,
unset by pcb-kicad). `pour_whole_rip_up` stays: it is on by default.

**3. Clippy** (x1182; +33 lines): pcb-router is clippy-clean
(`cargo clippy -p pcb-router --all-targets`): iterators for the indexed
loops, `is_multiple_of`, `as_chunks`, a slice for `stitching_vias`'
parent, type aliases (`FrameHook`, `SpokeCorridors`, `CellWindow`,
`SpokePad`, `RoutedItem`), `too_many_arguments` allowed on `plan_corridor`
with a reason, `neg_cmp_op_on_partial_ord` allowed where a NaN crossing
must count as not beyond.

**4. router.rs split** (x1183; pure moves, +324 lines of headers and
`use`): router.rs (15 395 lines at 54eff39, 14 291 before the split) keeps
the shared state and helpers in 920 lines; `router/` has config, result,
heap, wall, hash, profile, and `impl Router` in build, escapes, guards,
prepare, stamping, route_net, planner, search, update, batching, global,
driver, fixed, negotiate, pour_refresh, vias, finish, pour_analysis,
spokes, manual_spokes, pour_dump, stitch, cleanup, resolve, output; the
tests in router/tests.rs. Methods are `pub(super)`; the paths pcb-kicad
uses are re-exported (`router::{Config, Router, EXPANSIONS_PER_SECOND,
GUARD, wall_cuts, wall_guard, note_wall_cut, WALL_CUTS, build_totals,
congestion::tile_frame}`). router.md, "How it works", lists the files.

**Gates**, after every step: `cargo test --release -p pcb-router -p
pcb-kicad` (58 router tests, the dropped halves aside, and pcb-kicad's
365). Route mode on the v16 placements (`--route-from
build/github-layout-v16 --tasks build/github-tasks-v6/tasks.json --jobs 2
--threads 4`, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`) of
Castor, k30-SBC, jetson, Sisu, urchin and spell_tome, x1180, x1181, x1182
and x1184 (the four commits rebased on fb4b7e6) each against x1170: the
same board md5, vias, length, work seconds (1649.8, 1032.0, 418.5, 310.7,
16.8, 1.0), verdict and fingerprints on every board (`build/qb-x1170*`,
`build/qb-x118*`). `PCB_ROUTER_FINGERPRINT_ALL` at a quarter budget, two
threads, on Castor and k30-SBC, x1184 against x1170: identical
(`build/qb-fpall-*`). The starved-thermal census (census.py, fill3 KiCad
cache, 44 boards) 162 predicted / 7 missed / 26 false alarms / 2 not
modelled for x1170 and every step, the whole output identical
(`build/qb-census`). Short tier (tasks v6, 6 x 4 under the lock) x1184
against `build/short-x1170.log`: all 12 rows identical apart from CPU,
wall, load and memory (unconnected 19, new errors 2, 1134 vias, 21228 mm,
work 2501 s, passes 7, first-route open terminals 26).

Not pure clean-ups, for the reviewer: pcb-kicad's `jacobi_batch` field
(board_router.rs) and the `PCB_JACOBI_BATCH` line of src/main.rs stay but
do nothing now (the three lines that passed the field on were removed so
that pcb-kicad compiles); the verbose `profile:` line lost its speculation
counts and the `PCB_ROUTER_DEBUG` batch sizes their `j` marks; the
net-count assertion of `build` keeps its 15-bit limit, which the hard
guard's bit no longer needs.

## Committed on 2026-10-16 (quality: placer, topo, congestion)

A code-quality pass over `pcb-placer`, `pcb-topo` and `pcb-congestion`
with no behaviour change. Binary x1200 (this code; main had no Rust change
since 54eff39).

**Clippy.** `cargo clippy --release -p pcb-placer -p pcb-topo -p
pcb-congestion` reported 19 warnings in the three crates (6 + 11 + 2),
now none, also with `--all-targets` (the 26 left in that output are
`pcb-router`'s, a dependency):

- `needless_range_loop` (8): iterators with `enumerate` in
  `global.rs` (filler sides), `legal.rs` (`refine`'s bucket fill),
  `features.rs` (layer slots) and `layers.rs` (the layer DP, the
  stretches, the clusters, the stretch costs); same order, same values;
- `type_complexity` (7): aliases `Boxes` (anneal.rs), `NetPin`
  (global.rs), `Level` (lib.rs, the relaxation levels' tuple),
  `WireRoom` (layers.rs), `Ring` (realize.rs);
- `too_many_arguments` (2, `realize::pull`, `cached_crossing_cost`):
  allowed, as elsewhere in the crates;
- `chunks_exact_to_as_chunks`, `unnecessary_sort_by` (a stable
  `sort_by_key` on the same key), `manual_is_multiple_of`: as suggested;
- with `--all-targets`, `excessive_precision` in the tiny model's test:
  allowed, the expected values are PyTorch's as printed.

No NaN-sensitive comparison was among them.

**File sizes.** No `pcb-placer` file is over 2 500 lines (the largest:
legal.rs 1 713, constraints.rs 1 277, lib.rs 1 119), so nothing was split.

**Kept on purpose** (documented, off or parked), untouched:

- `shared_halos` (placer option, off): the field `Problem::shared_halos`
  (problem.rs:147), its branches in lib.rs:386, global.rs:297,
  anneal.rs:225, legal.rs:482, room.rs:186; the KiCad option in
  board_placer.rs:143;
- `anneal_far_side_gaps` (off): board_placer.rs:146 sets
  `AnnealConfig::legal_far_side`; the annealer's `far_overlap` and
  `far_courtyards` (anneal.rs:374-479, about 106 lines) and `build_state`;
- `routing_room` (off): crates/pcb-placer/src/room.rs (428 lines),
  `Config::routing_room` in lib.rs (about lines 967-1063),
  crates/pcb-kicad/src/placement_room.rs (676 lines);
- the topological engine: crates/pcb-topo (8 files, 5 730 lines), called
  from board_router.rs (around line 5159), docs/topological.md;
- the congestion network: crates/pcb-congestion/src/model.rs (190 lines),
  crates/pcb-kicad/src/congestion_rank.rs (225 lines);
  crates/pcb-congestion/src/features.rs (790 lines) also holds
  `rudy_overflow`, which the race uses.

**Identity.** Tests (`-p pcb-placer -p pcb-topo -p pcb-congestion -p
pcb-kicad`, release) pass. `place-kicad-board` on the short tier's prepared
sources (build/short-x1170, with each board's layout.json, 4 threads):
spell_tome_bottom, USB_Cable_Tester, urchin, eternal-keypad, OpenFC and
Blue_Line give byte-identical boards with x1170 and x1200.
`score-congestion` (q-ensemble.onnx) on urchin's and OpenFC's placements:
the same `open`, `overflow` and tiles. Route mode on Castor (v16
placement, quarter budget, 4 threads, `PCB_ROUTER_FINGERPRINT=1
PCB_ROUTER_FINGERPRINT_ALL=1`): the same four fingerprints in the same
order, the same routed board (md5) and row (126/126, 187 vias, 5490 mm,
work 150.4 s).

## Committed on 2026-10-16 (stamp)

`Router::stamp` made cheaper with routes and occupancy maps byte-identical
(the next target the search-core round named). Binaries: x1140 (the change
on aacdd28), x1141 (the same without a no-op, see below), x1142 (on
015676c, for the last pairs), x1143 (the commit's code on 28b9bf8, for
the short tier). Scripts and logs in the worktree
`pcb-maker-stamp` (`build/prof`, `build/bench`, `build/check`,
`build/fpall`, `build/pairs`).

**Line profile, before** (x1123's code with line tables, gperftools at
200 Hz, route mode on the v16 placements, one search thread, share of the
process's samples; kicad-cli did not run under the profiler). `stamp` is
10.5 % of Castor, 6.0 % of k30-SBC and 9.8 % of Sisu; `unstamp`,
`mask_own` and `OwnStamps::new` together 0.1-0.3 % (`route_jacobi`'s
unstamp is rare: few parallel batches). Its two phases, Castor / k30 /
Sisu:

| what | Castor | k30 | Sisu |
|---|---|---|---|
| list building: per node and querying class, every offset of four footprints (trace, via, two diagonal-block maps) tested against the mark bitset | 4.2 | 2.5 | 3.5 |
| of it: the mark test / bounds tests / loop / push | 2.5 / 0.7 / 0.4 / 0.4 | 1.6 / 0.4 / 0.3 / 0.3 | 2.1 / 0.5 / 0.4 / 0.3 |
| diff (the two rows below) | 6.2 | 3.4 | 6.2 |
| diff: old list (mark test, count down, tile claim) | 0.6 / 2.0 / 0.2 | 0.4 / 1.0 / 0.1 | 0.7 / 2.0 / 0.2 |
| diff: new list (mark test, count up, tile claim) | 0.6 / 2.2 / 0.3 | 0.4 / 1.2 / 0.2 | 0.6 / 2.2 / 0.2 |

What the lists hold (Castor, a counting build, one route): 18 000 calls,
775 M old stamps, 818 M new, 565 M of them added and about 522 M removed:
two thirds of every list changes, because the negotiation calls `stamp`
twice per reroute (`rip_up_conflicted` takes the conflicted branches'
stamps away, the stamp after the search puts the new ones in), so the
restamp diff saves little there and the count changes themselves are the
cost. The count lines are memory: Castor's 42 occupancy maps are 1 MB
each (568 x 865 cells of u16), beyond the core's 512 KB L2.

**Changed** (crates/pcb-router/src/router.rs, `Fresh`, `mark_footprint`,
`prefetch`, `Router::stamp`). Measured with bench-search as two processes
on the two hyperthreads of one core, sides swapped each repetition
(build/bench/sib.sh: user seconds, the bench lines must agree), Castor,
12 iterations, 6 repetitions; null test of x1123 against itself 1.000
(0.998-1.001):

- Leading edges (0.964 against x1123): a node one step from the node
  before (8-neighbour or the same cell), in the same class and, for the
  trace and diagonal-block maps, on the same layer, marks only the
  offsets of its footprint that the node before did not cover: `Fresh`,
  per footprint and step, the offsets `o` with `o + step` not in the
  footprint, in the footprint's order. Every skipped cell was marked by
  the node before, which applied the same footprint to the same map, so
  the list gets the same cells in the same order. A disc of radius r
  goes from about 3r^2 offsets to 2r+1 on a straight run. With it: no
  bounds test when the node is at least the footprints' reach from the
  edges, the node classes in one flat list (was a vector per branch), and
  a via through several layers at one cell stamps its via footprints
  once. (The diagonal bisectors cannot use it: the bisectors of a
  diagonal run are parallel lines that share no cell; x1140 carried that
  case, x1141 dropped it, 1.003 against x1140, identical lists.)
- Prefetch in the diff (0.982 against the step before, 16 entries
  ahead; 32 ahead against 16: 0.998): the count of the stamp 32 entries
  on is asked for early (`_mm_prefetch`; nothing off x86_64).
- One flat mark bitset for all maps instead of a vector per map: 0.998.
- Tried and dropped: tile claims without a branch and without a division
  (multiply-high by a reciprocal): 1.005; division-free with the branch:
  1.002; prefetching the mark word too: 1.004.

All three against x1123 (bench-search, the whole process including the
lattice build): Castor 0.947 (12 iterations), Sisu 0.958 (4), k30-SBC
0.975 (4), jetson 0.985 (2). bench-search spends more of its time
stamping than route mode (15.8 % of the Castor bench before, 7.6 % after).

**Line profile, after** (the change on x1123's code, same settings; a
quieter machine: load about 10 against 15-25 before, so the seconds are
not a claim): `stamp` 3.6 % of Castor (list 1.4, diff 2.2), 2.3 % of k30
(0.9, 1.4), 3.0 % of Sisu (1.6, 1.4); fingerprints identical to the
before-runs. Remaining lines: the mark test of the leading edge (0.6 on
Castor), the count tests (0.4 each way), the push (0.2).

**Byte-identical.** A check build ran the original list building beside
the new one on every call and asserted equal lists (content and order):
route mode, two threads, urchin, spell_tome, Castor, k30-SBC, Sisu and
jetson, about 185 000 calls, all equal. `PCB_ROUTER_FINGERPRINT_ALL`
(every occupancy map hashed into each fingerprint) at a quarter of the
budget, two threads, x1132 against x1141: urchin, spell_tome, Castor,
k30-SBC, Sisu and jetson identical (fingerprints and board md5,
`build/fpall/fpall.out`). Unit test `fresh_footprints_mark_as_whole_ones`
(random walks near the edges: the same list and marks with leading
edges; bisectors share no cell).

**CPU, pinned simultaneous pairs** (route mode, `--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1`, `cpu_seconds`; route mode on fixed placements is
untouched by the placer and calibration commits in between, so both
bases hold for the ratio). r1, r3: the base on 0-7,16-23; r2, r4
swapped:

| board | r1 x1132 / x1141 | r2 swapped | r3 x1152 / x1142 | r4 swapped | all four |
|---|---|---|---|---|---|
| Castor | 0.946 (257.9 / 244.0) | 0.966 (277.2 / 267.9) | 0.923 (336.3 / 310.5) | 1.011 (319.6 / 323.1) | 0.961 |
| k30-SBC | 0.934 (401.8 / 375.2) | 1.007 (306.0 / 308.1) | 0.950 (536.1 / 509.5) | 1.023 (489.3 / 500.7) | 0.978 |
| Sisu | 0.969 (1185.0 / 1148.4) | 1.004 (876.7 / 880.3) | 0.925 (1151.3 / 1064.5) | 1.016 (1092.7 / 1109.8) | 0.978 |
| jetson | 0.944 (1451.5 / 1369.7) | 1.050 (1095.1 / 1150.2) | 0.911 (1452.7 / 1323.4) | 1.024 (1382.9 / 1415.8) | 0.981 |
| geomean | 0.948 | 1.006 | 0.927 | 1.018 | 0.974 |

Ratios new / old. The halves differed by about 8 points this time (the
new binary on 8-15 gained 5-7 %, on 0-7 it lost 1-2 %), more than the 3-5
of earlier rounds, so only the four together say anything: about 2.5 %
of route mode's CPU, the stamping's share falling by two thirds. Every
pair: the same board file (md5), vias, length, work seconds, verdict and
fingerprints (`PCB_ROUTER_FINGERPRINT=1`).

Short tier (tasks v6, tier default 8 x 2, under the lock) x1143 against
`build/short-x1161.log`: all 12 rows identical apart from CPU, wall,
load, memory and the designer DRC's seconds (routed, KiCad's findings,
vias, length, work seconds, first-route open terminals; the tasks path
spelled relative, the same sha256); compare.py same 12, unconnected 19
to 19, passes 7 to 7, cut 0. Its CPU (2882 to 2784 s) is not a claim:
unpaired.

`cargo test --release -p pcb-router -p pcb-kicad` passes.

Next on this path: the counts. The negotiation takes a rerouted branch's
stamps out (`rip_up_conflicted`) and puts the new route's in after the
search; two thirds of the stamps change on every call, and a branch that
comes back on the same cells pays both. Leaving the ripped stamps in
place and masking them in the search like the kept branches (`mask_own`
and `own_claimed` already do this for the kept ones) would let one diff
cancel them, but every reader of the occupancy during the search (the
tile planner, the conflict and history bookkeeping, pour checks) must see
the same values, a semantic change not attempted here.

## Committed on 2026-10-16 (order independence)

The defect: the placement, and through it the layout, depended on the
order of the footprints in the `.kicad_pcb` and on the order of each
footprint's own items, which pcbnew's save, the import and the schematic
flow all permute; two saves of one design laid out apart. Binaries x1160
(on aacdd28, the order alone), x1161 (on 015676c, the order and the exact
quarter turns: the measured one). Runs in `build/order-independence/`,
tools in `experiments/order-independence/` (`permute.py`: a project
with its footprints, and each footprint's pads and graphics, as is,
reversed or shuffled with seed 7; `compare.py`: poses by reference and
uuid, placed and routed board and copper identical or not; `run.sh`).

**Change 1, a canonical order as the board is read**
(`canonical_order::parse_board` in pcb-kicad, used by `place-kicad-board`,
the layout's outline sizing and reads of the placed board, every router
attempt (`route_kicad_board_once`, the poured copy), the congestion score
and the designer diagnostics): footprints by reference in natural order,
then uuid, then their text; in each footprint the pads by number
(natural), then position, the `fp_*` graphics by layer and content
without uuid, the footprint's zones by content; each kind keeps the
places it held. Nothing in `pcb-placer` changed: with one input order,
global placement, the annealer's draws, the legaliser, refinement, rows,
channels, net numbering, the pool's seeds and the race's probes see one
order. The boards written are in that order, so the result file is
byte-identical from any input order (a side effect: the placed and routed
boards no longer keep the input's footprint order; KiCad does not mind,
its own save sorts by uuid).

**Change 2, exact quarter turns** (`pcb_router::geometry::degrees_sin_cos`,
in the placer's pad boxes and the router's pad shapes, rotations and
rectangles): found on the orientation search's pcbnew-saved pair
(homebrew_486 from 0 degrees and from the designer's angles): with the
order alone (x1160) it placed 102 of 130 parts elsewhere, since pcbnew
writes a pad's absolute angle wrapped (relative -90 in one file, 270 in
the other; footprints at -180 or 180) and `to_radians().sin_cos()`
rounds them apart (1e-16 in a pad box); x1132 had been lucky in pcbnew's
order. With both changes the pair gives the same poses and the same
copper (971 vias, 0 unconnected).

**Three orders** (schematic tier's prepared sources; placement alone with
200 s of work and three seeds; layouts at a third, four threads; parts
elsewhere against the file as is; vias, length mm, KiCad unconnected):

| Board | x1132 placement (reversed, shuffled) | x1132 layouts (as is / reversed / shuffled) | x1161 placement | x1161 layouts, all three |
| --- | --- | --- | --- | --- |
| urchin | 20, 22 of 122 | 41, 2683, 0 / 41, 2692, 0 / 40, 2603, 0 | identical | 43, 2636, 0, byte-identical |
| coco2 | 148, 148 of 156 | 1144, 26,093, 0 / 593, 25,126, 0 / 852, 24,226, 3 | identical | 932, 27,715, 0, byte-identical |
| homebrew_486 | 104, 104 of 130 | 1008, 27,273, 0 / 1033, 27,657, 0 / 1039, 27,226, 0 | identical | 971, 26,869, 0, byte-identical |

x1160 (before the calibration) was byte-identical across the orders too
(41, 1001, 959 vias). coco2's 593-1144 vias is the order alone; the
schematic tier's coco2 rows were draws from that spread.

**Short tier** x1161 against `build/short-x1151.log` (6 x 4, the lock):
passes 6 to 7, unconnected 30 to 19, new errors 3 to 2, vias 1178 to
1134, length 21,528 to 21,228 mm, work 2327 to 2103 s, CPU 4143 to 2882
s, cut 0; better 3 (OpenFC 2 to 0 unconnected, pass; reCamera 9 to 4;
MIDAS-MK1.1 11 to 7, 246 to 212 vias), same 9, worse 0. Every placement
changed (the stack holds the designer's order): vias per row
eternal-keypad 56 to 65, USB_Keypad 106 to 111, OpenFC 154 to 159,
reCamera 68 to 76, spell_tome 8 to 9, USB_Cable_Tester 90 to 128,
Blue_Line 25 to 26, MIDAS 246 to 212, 0xCB-1337 78 to 57, urchin 40 to
42, WLED_Matrix 216 to 157 (errors 2 to 1), framework_mobo 91 to 92; a
reshuffle like the orientation change's, this time a good draw.
**Schematic tier** x1161 against a fresh x1152 base on the same tasks
(`build/sch-x1152-order-base.log`; `build/sch-x1130.log` predates the
calibration): passes 8 to 10 (OpenFC, TelemetryOnboard), unconnected 6
to 4, vias 3027 to 3184, length 91,198 to 92,388 mm, work 3164 to 2931 s.
Identical rows and placements: urchin, spell_tome, USB_Keypad,
0xcb-static, bumwings (their import was already in reference order, so
the exact quarter turns changed nothing there either). Reshuffled
placements: coco2 717 to 932 vias, homebrew_486 960 to 971, OpenFC 165
to 148 (2 to 0 unconnected), stm32_chiffre 62 to 112, MokyaLora 420 to
400. Same placer seeds, other routing: slab-pcb-left (the race's probes
9 / 185 / 4 against 8 / 9 / 12 unfinished keep another seed: 405 to 326
vias, 1 to 2 unconnected) and TelemetryOnboard (one part moved by the
layout's moves, 1 to 0 unconnected).

**Threads:** Blue_Line, spell_tome and 0xCB-1337 (short tier) at 1, 2 and
4 threads: identical rows and byte-identical boards (the calibration
agent saw the same on urchin, USB_Cable_Tester, reCamera).

Tests: `canonical_order` (natural order; footprints by reference then
uuid, other items in place, idempotent; pads and graphics from three
orders), `board_placer::order_tests` (the lowering and a whole placement
identical from a file and its reverse; both fail without the sort),
`geometry::one_angle_spelled_two_ways_has_one_sine_and_cosine`.
`cargo test --release -p pcb-kicad -p pcb-placer -p pcb-router`,
`cli_exit_codes`, `agent_walkthrough` pass.

Left: the board's top-level graphics, zones, tracks and net list stay in
the file's order (nothing measured followed them); a save that rounds
coordinates differently is another input.

## Committed on 2026-10-16 (calibration and tier parallelism)

Two measurements, one constant and one runner default. Binaries: x1150
(main 8e8b4ba plus the `profile rate:` line, for the rates), x1151 (main
aacdd28 plus the new constant: the commit's code). Scripts and logs in the
worktree `pcb-maker-calib` (`build/rate`, `build/det`, `build/par`); tier
logs `build/short-x1151*.log` in the main tree.

**`EXPANSIONS_PER_SECOND` 2.0e6 to 3.15e6.** Measured at one search
thread (`RAYON_NUM_THREADS=1`) in route mode on the v16 placements, two
boards at a time, each pinned to eight physical cores (`taskset -c 0-7`
and `8-15`), started at a load under 10 (10-17 while they ran, other
agents included). New verbose line `profile rate:` per router: the
phases' expansions over their wall seconds without the pour analyses
(what a work second stands for in the phase budgets), the pour analyses'
work and wall, and with `PCB_ROUTER_SEARCH_RATE=1` the rate inside
`search` alone (it times every search; off, a branch per search):

| board | routers | expansions | phase s | per second | inside `search` | pour work / wall |
|---|---|---|---|---|---|---|
| Castor | 1 | 611.7M | 183.4 | 3.34M | 3.81M | 29.1 / 19.4 (1.50) |
| jetson | 5 | 3555.5M | 1101.2 | 3.23M | 3.57M | 16.4 / 16.1 (1.02) |
| k30-SBC | 1 | 638.5M | 195.2 | 3.27M | 3.58M | - |
| Sisu | 5 | 2458.5M | 801.4 | 3.07M | 3.49M | 33.6 / 30.6 (1.10) |
| urchin | 1 | 56.2M | 20.3 | 2.78M | 3.17M | - |
| spell_tome | 1 | 2.3M | 1.0 | 2.27M | 3.27M | 1.6 / 1.7 (0.93) |

Median 3.15M (Sisu 3.07, jetson 3.23): the constant is 3.15e6, 1.57 times
the old one, not the 5-10 % the search-core note guessed (that was one
speed round; the 2.0e6 was an eight-core negotiation's rate from before
all four rounds, and the one-thread rates the core agent saw were bench
runs under load 14-30 with lattice builds included). A budget in seconds
now buys 1.57 times the expansions, and on the reference a work second
is a second of the phases' wall clock (it was 0.6-0.7 s). The pour model
is left: work / wall 0.93-1.10 but Castor's 1.50 (idle, its analyses ran
faster than in their loaded calibration of x1013). Unit test
`a_budget_in_seconds_buys_expansions_at_the_calibration` (a probe's limit
is its seconds at the constant; work, the profile's phase expansions and
the `profile rate:` line count the same expansions). The placement
pool's `REFERENCE_THREADS` (8) is the placement's own convention, not the
router's calibration; its comment and docs/router.md say so now.

Short tier, `--jobs 6 --threads 4` under the lock, x1151 against
`build/short-x1130.log` (x1132's tree, the coordinator's reference):
compare.py better 1, same 11, worse 0; unconnected 32 to 30, passes 6 to
6, new errors 3 to 3, vias 1130 to 1178, length 20777 to 21528 mm, work
2840 to 2327 s, CPU 3116 to 4143 s, cut 0 to 0, first-route open
terminals 59 to 52; wall 554 to 482 s (load differed). Eleven rows
identical in routed, KiCad's findings, vias, length and first-route open
terminals; their work seconds fall (eternal-keypad 177 to 118, OpenFC 584
to 394, USB_Keypad 451 to 288: the same expansions at the higher rate)
and their CPU rises where a budget ran out (reCamera 397 to 662 s, WLED
614 to 778: more search for the same budget, the same board). One row
moved, by more work per budget: MIDAS-MK1.1 149/156 to 150/156, KiCad
unconnected 13 to 11, vias 198 to 246, 3421 to 4173 mm, first-route open
22 to 15 (its first route negotiated with 1.57 times the expansions).
Wall clock: a layout now spends about its budget in wall time on an idle
core, so the wall guard (1.1 times) has much less room than before under
load; no row was cut in any run below (load 9-25), but a loaded machine
(v16's load 16-35 at five jobs of four threads) would cut rows that the
old constant's 35 % slack hid. Keep tiers at the default parallelism and
under the lock.

**Thread independence, checked.** urchin, USB_Cable_Tester and reCamera
laid out with x1151 at 1, 2 and 4 threads (`--tier short --only ...
--jobs 3`, `PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1`):
prepared source boards, placed boards and result boards byte-identical
(md5), rows identical (work 60.7 / 119.1 / 385.2 s at every count). The
fingerprints match on urchin and USB_Cable_Tester; on reCamera at 4
threads the ladder's last 0.05 mm wave ran two rungs side by side where
1 and 2 threads ran one, and the second (fingerprints `46eced3e...`,
`27ace45f...`) was dropped unjudged at the deadline: CPU spent, nothing
judged or counted (the memory free decides how many rungs run side by
side, not which are judged). And the whole short
tier at 6 x 4, 16 x 1, 8 x 2 and 4 x 4 gave the same 12 rows.

**Tier parallelism** (x1151, short tier, one run each under the lock,
started at load 9-12, mem 21-29 GB):

| jobs x threads | wall s | CPU s | board s summed | WLED_Matrix s | cut |
|---|---|---|---|---|---|
| 16 x 1 | 448 | 3562 | 2732 | 433 | 0 |
| 8 x 2 | 401 | 3263 | 2006 | 339 | 0 |
| 4 x 4 | 570 | 3305 | 1877 | 327 | 0 |
| 6 x 4 (above) | 482 | 4143 | 2258 | 406 | 0 |

The tier is as long as its longest board, WLED_Matrix (the runner starts
it first), and a board gains from a second thread (the placement pool
runs its seeds on the search threads; the negotiation's few parallel
batches) but little from more, while 16 x 1 shares the cores so that
every board runs slower (2732 board seconds against 2006). Chosen: 8 x 2,
run.py `TIER_THREADS = 2` with physical cores / 2 jobs as the default of
every tier (quick, short, schematic) when neither `--jobs` nor
`--threads` is given; either given keeps the old defaults for the other.
`summary.json` records `parallelism` ("tier default" or "command
line") beside `jobs`, `threads`, `physical_cores`; README "How to measure"
says rows compare across thread counts while the check above holds.
`benchmarks/agent-tasks/test_run.py` (4 tests). One run per
configuration: 16 x 1 against 8 x 2 is 47 s, within what the load can
move; the quick tier (30 boards, more throughput than critical path) was
not measured and may prefer 16 x 1.

`cargo test --release -p pcb-router -p pcb-kicad` passes.

## Committed on 2026-10-15 (KiCad check work)

Item 4 of the v16 triage's ranked list, second half: KiCad's checks inside
a layout (the board kept after the moves and the polish, the final
ladder's checked attempts, the neck repair's checks and its KiCad fill)
cost 3 to 60 s of wall each uncached and under a second from the DRC
cache, and none of it was work, so the layout's end in work came at wall
times that followed the cache and the board. Binaries: x1080 (the change
on 84fcc3b, first calibration), x1081 (rebased on aacdd28, recalibrated,
budget-guard floor), x1082 (rebased on 015676c), x1083 (on 28b9bf8,
the short tier), x1084 (on 1a5e0f6, the commit's code).

**The charge** (check_work.rs; docs/router.md "KiCad's checks as work").
Every check counts a cost modelled from the board file alone, read before
the layout starts: `CHECK_SECONDS` (3.6 s) plus
`SECONDS_PER_THOUSAND_HATCH_HOLES` (1.7 s) per thousand holes of the
board's hatched copper zones (zone area on each copper layer over the
square of hatch thickness plus gap), at most 50 s. It does not depend on
whether the cache answered, so a warm run makes every decision a cold run
makes. It goes into the ladder's single work tally (`ladder_work_now`: the
ladder's budget, `spent`, `deadline_work`), into the layout's `work`
(the kept board's check, before the final ladder's deadline is taken) and
into `routed.work_seconds` (the neck repair after the ladder). The
reservation arithmetic keeps the step's own check: the narrow-signal
reserve is the step's routing reserve plus one check (`reserve_with_check`),
a rung or seed round starts only if its projected work and its check leave
the reserve whole (`fits_before_reserve(work_left - check, ...)`, the seed
rounds' `affordable`), and every router's work deadline ends one check
before the ladder's end (rungs, seeds, the probed rung's continuation, the
step: `routing_left`). The continuation's seeds multiply their routing,
not their checks (`seeds_work`). Route mode (no layout) charges nothing
and is unchanged. Reported: `layout: a KiCad check counts S s of work (N
hatched zones, H hatch holes)` at the start, `check work N x S s = T s`
on `layout: done`, `kicad_check_seconds`, `kicad_checks_charged`,
`kicad_check_work_seconds` in board-layout.json (and the last two per
ladder in board-router.json), `kicad_check_work_seconds` in the verdict
and run.py's rows. Not reserved: the first route, moves and polish are
still armed at the layout's end, so a layout that spends all of it before
the ladder ends one check past its budget in work (the check is counted,
the wall guard has its 10 %).

**Measured check times.** v16 sweep logs (x862, uncached `board check`
lines over 2 s): 191 checks on 59 boards, pooled median 9.3 s (mean
15.6); per board, the 51 boards without hatched zones 8.0 s (quartiles
6.2 / 14.2), the 8 hatched 29.8 s (4.8 to 49.4). The short tier's cold run
(build/drccache/short-cold, x801): 53 checks, pooled median 9.4 s, plain
boards 5.6 s, hatched 23.8 s. Every routed v16 board and short-tier board
checked again one at a time, uncached (`drc-kicad-board`,
PCB_MAKER_NO_CACHE=1): x978 at load 20-48, plain median 5.3 s (58 boards);
x1093 (library-free kicad-cli config) at load 14-50, plain median 3.6 s
(quartiles 2.8 / 7.2, slowest 28 s), 0.69 of x978 per board; a spot check
of 23 boards on x1132 found no change beyond the load (the same board 32 s
at load 10, 55 s at load 21). The slow plain boards are ULK (27 s), koeg
(26-33 s), ghoul (18-22 s): complex outlines, which the model does not
see. Hatch holes explain the hatched boards: least squares 1.73 s per
thousand on x1093 (1.57 on the v16 logs), correlation 0.90 between model
and measured on the nine hatched boards (Batch_1 30500 holes 60 s at load
18-36, qfwfq 14500 22 s, capybully 10600 25 s, the 0xCB panel 8100 in
four zones 29 s, WLED_Matrix 7200 14-34 s with the load, USB_Keypad 6600
9 s, stm32 chiffre 6800 on one layer 4 s, 0xCB-1337 2000 2 s). Over all
boards the model's median relative error is 0.48 (as a constant's: the
plain boards' spread), its summed absolute error 308 s against 384 s for
a constant. A first fit read only the first points of each zone polygon
(half of every rectangle's area) and gave 2.7 s per thousand; x1080 has
that, with 7 s a check (the pre-x1093 median).

**Judge, eight boards, layout mode, 4 threads, one run per side, both
sides side by side** (x978 against x1080 on 84fcc3b; rows in
build/checkwork/judge-{x978,x1080}-{cold,warm}.log). Cold is an empty cache
directory of the run's own (`PCB_MAKER_CACHE_DIR`), so every check runs
KiCad and fills it (0 hits in every cold row); warm reuses it. unconnected /
errors / vias / work / wall / cut / KiCad checks (hits, kicad-cli seconds) /
charged check work:

| board | x978 cold | x978 warm | x1080 cold | x1080 warm |
|---|---|---|---|---|
| ghoul | 6 / 0 / 385 / 1152 / 962 / no / 3 (0, 86 s) | same board, 1152 / 775 / no / 3 (3, 2 s) | 6 / 0 / 385 / 1043 / 869 / no / 3 (0, 74 s) / 21 | same board, 1043 / 640 / no / 3 (3, 2 s) / 21 |
| 0xCB panel | 118 / 0 / 248 / 1191 / 1113 / no / 4 (0, 176 s) | same, 1191 / 758 / no / 4 (4, 3 s) | 118 / 0 / 248 / 1108 / 929 / no / 4 (0, 154 s) / 116 | same, 1108 / 702 / no / 4 (4, 2 s) / 116 |
| Batch_1 | 491 / 0 / 384 / 1471 / 1158 / no / 2 (0, 114 s) | same, 1471 / 1376 / no / 2 (2, 3 s) | 491 / 0 / 384 / 1359 / 1110 / no / 2 (0, 108 s) / 100 | same, 1359 / 1380 / no / 2 (2, 3 s) / 100 |
| ULK | 4 / 0 / 130 / 1478 / 1616 / no / 7 (0, 196 s) | same, 1478 / 1285 / no / 7 (7, 5 s) | 4 / 0 / 130 / 1527 / 1496 / no / 7 (0, 218 s) / 49 | same, 1527 / 1293 / no / 7 (7, 6 s) / 49 |
| woagboard | 5 / 1 / 185 / 1540 / 1395 / no / 8 (0, 60 s) | same, 1540 / 1406 / no / 8 (8, 5 s) | 5 / 1 / 185 / 1596 / 1547 / no / 8 (0, 64 s) / 56 | same, 1596 / 1658 / deadline,layout / 8 (8, 5 s) / 56 |
| MokyaLora | 0 / 0 / 333 / 1206 / 634 / no / 3 (0, 21 s) | same, 1206 / 660 / no | 0 / 0 / 333 / 1227 / 616 / no / 3 (0, 22 s) / 21 | same, 1227 / 666 / no / 21 |
| zpn_devboard | 0 / 0 / 417 / 522 / 512 / no / 1 (0, 7 s) | same, 522 / 503 / no | 0 / 0 / 417 / 529 / 521 / no / 1 (0, 6 s) / 7 | same, 529 / 499 / no / 7 |
| Castor | 0 / 0 / 167 / 924 / 650 / no / 2 (0, 9 s) | same, 924 / 754 / no | 0 / 0 / 167 / 938 / 686 / no / 2 (0, 9 s) / 14 | same, 938 / 773 / no / 14 |

The kept board (the result's .kicad_pcb, byte for byte) is the same on all
eight boards across the four runs, and each binary's work is the same warm
and cold. Where x1080's work is lower the charge ended the ladder earlier,
dropping only attempts that were not checked and could not be kept: ghoul
one 0.075 mm skeleton rung (router count 14, cannot win), the panel the
0.075 mm skeleton and tracks rungs (unchecked), Batch_1 the seed retry and
the tracks rung after its skeleton rung (Batch_1's 50 s checks); ULK's and
woagboard's ladders ran the same attempts, the finer pitches with a little
less negotiation (woagboard's unchecked 0.05 mm rungs 79 / 90 open by the
router against 75 / 86). Cut: none, but woagboard warm on x1080: the wall
guard (1650 s) fired in its last rung and skipped the neck repair, which
had nothing to keep on this board (the same board as the cold run); that
run's search was slower than its work under the load (1658 s of wall for
1596 s of work; its cold twin 1547 s), not its checks (5 s of kicad-cli).
The checks' uncounted wall on the cold runs, summed: x978 669 s of
kicad-cli, none counted; x1080 655 s, 384 s counted (ghoul 74 / 21, panel
154 / 116, Batch_1 108 / 100, ULK 218 / 49, woagboard 64 / 56). Wall minus
work over the eight cold rows went from -1443 to -1552 s; per board it
shrank where the uncounted checks were largest (the panel -78 to -179,
ULK +138 to -31) and did not on ghoul, Batch_1 and woagboard (-190 to
-174, -314 to -249, -145 to -48), whose ladders changed and whose wall
follows the load (work above wall: seeds and probes side by side count
each router's work).

**Judge again on the 3.15e6 calibration** (x1152, main 015676c, against
x1082, the change on it; cold as above, same conditions; rows in
build/checkwork/judge-{x1152,x1082}-cold.log). A work second is now a wall
second of an idle core, so the uncounted checks are a larger share of the
slack:

| board | x1152 cold | x1082 cold |
|---|---|---|
| ghoul | 6 / 0 / 338 / 1144 / 1189 / no / 6 (0, 125 s) | same board, 1166 / 1188 / no / 6 (0, 129 s) / 22 |
| 0xCB panel | 97 / 0 / 242 / 1145 / 1075 / no / 5 (0, 133 s) | same board, 1117 / 1080 / no / 5 (0, 137 s) / 87 |
| Batch_1 | 471 / 0 / 388 / 1492 / 1752 / deadline,layout / 4 (0, 132 s) | 438 / 0 / 508 / 1461 / 1495 / no / 3 (0, 103 s) / 150 |
| ULK | 2 / 0 / 121 / 1066 / 1371 / no / 7 (0, 162 s) | same board, 1091 / 1322 / no / 7 (0, 163 s) / 25 |
| woagboard | 16 / 0 / 172 / 1196 / 1673 / deadline,layout / 7 (0, 41 s) | same board, 1113 / 1455 / no / 6 (0, 36 s) / 22 |
| MokyaLora | 0 / 0 / 341 / 1241 / 936 / no / 3 (0, 14 s) | same board, 1252 / 936 / no / 3 (0, 14 s) / 11 |
| zpn_devboard | 0 / 0 / 367 / 453 / 495 / no / 1 (0, 4 s) | same board, 457 / 498 / no / 1 (0, 4 s) / 4 |
| Castor | 0 / 0 / 207 / 610 / 559 / no / 2 (0, 4 s) | same board, 617 / 562 / no / 2 (0, 4 s) / 7 |

Cut 2 to 0. On x1152 the wall guard cut Batch_1 in its 0.05 mm rung (181
s of that router's work, three checks of 30-36 s before it uncounted) and
skipped the neck repair; x1082 counts 50 s a check there, ends the ladder
after the 0.075 mm pitch by work and keeps another board, 438 unconnected
against 471 (508 vias against 388; the cut run's board is the one that
depends on the load). woagboard: x1152's guard cut its last 0.05 mm
tracks rung and skipped the neck repair; x1082 skips one check and ends
by work, the same board. The six other boards keep their board; the work
rises by the charge (ghoul, ULK, MokyaLora, zpn, Castor); on the panel
the 0.075 mm seed retry no longer fits with its check before the reserve
and a 0.05 mm rung runs instead (unchecked, 110 open by the router), work
1145 to 1117 s. Wall minus work over the eight rows
703 to 263 s (Batch_1 261 to 34, woagboard 478 to 342, ULK 305 to 231,
ghoul 45 to 22). The warm runs of this pair were stopped for time; warm
against cold was checked on x1080 above (same boards, same work).

**Short tier** (flock, tasks v6, `--jobs 2 --threads 4`), x1081 against
build/short-x1130.log (main aacdd28, same thread count): all 12 rows
identical (unconnected 32, new errors 3, 1130 vias, 20777 mm, passes 6,
cut 0). Work 2840 to 3139 s: each row rises by its charge (3.6 s a check;
7.0 s on 0xCB-1337, 2000 hatch holes; 15.9 s on WLED_Matrix, 14.9 s on
USB_Keypad) except two whose ladders ended earlier: WLED_Matrix +72 s for
159 s charged (8 checks: the ladder stopped before the 0.075 mm pitch,
whose two rungs were not checked, 7 and 8 open by the router, and so
could not be kept), USB_Keypad +161 s for 342 s charged (21 checks,
15 of them the neck repair's after the ladder; the ladder stopped before
the 0.05 mm seed retry and tracks rung, both unchecked). KiCad checks the
same on every row (41). build/short-x1081.log.

Again on main 28b9bf8 (x1083, the change on it) against
build/short-x1161.log (x1161, same thread count): all 12 rows identical
(unconnected 19, new errors 2, 1134 vias, 21228 mm, passes 7, cut 0).
Work 2103 to 2501 s, every row by exactly its charge (3.6 s a check; 7.0
on 0xCB-1337, 15.9 on WLED_Matrix's 9 checks = 159 s, 14.9 on
USB_Keypad's 10 = 179 s); no ladder ended earlier on these placements;
KiCad checks the same on every row (35). build/short-x1083.log.

**Budget guard floor** (router.rs, `BUDGET_GUARD_FLOOR` 5 s,
`budget_guard_fires`): in route mode a phase budget is cut by the wall
clock at four times the budget, so a budget under about half a second was
cut before its first pour analysis (a second of wall on Batch_1: "budget
guard: 0 s of wall clock for a budget of 0 s of work"); the guard now
waits at least 5 s. Unit test
`a_tiny_budget_has_the_guard_floor_before_its_wall_guard`. Layouts are
unaffected (their routers have a deadline, which replaces this guard).

Unit tests: check_work.rs (the constant on a board without hatch, hatch
holes from area, layers and pitch, the cap, `checks_count_the_same_work_cached_or_not`),
`the_reserve_covers_the_steps_own_check` (board_router.rs),
`seeds_multiply_their_routing_not_their_checks` (board_layout.rs);
`cargo test --release -p pcb-kicad -p pcb-router` passes.

Not done: the model does not see what makes ULK, koeg and ghoul slow
(complex outlines, 18-33 s a check at 3.6 s charged); a term from the
outline's corner count would be the next input to try. The first route,
moves and polish do not reserve the kept board's check.

## Committed on 2026-10-15 (orientation search)

The premise was open problem 9 item 8: the import puts every part at 0
degrees, the layout tier's stack keeps the designer's angles, and "the
placement follows the orientations it starts from" (coco2: 549 vias from
the designer's angles, 1144 from 0). Measured, it does not follow them:
the placer turns parts at every stage already (global placement's
`choose_rotations`, the annealer's quarter turns, the legaliser and
refinement try every angle), and from the designer's angles x1112 left
only 25-40 % of the moved parts at them (22-35 % from 0 degrees, a quarter
by chance; ten short-tier boards, four seeds). What the start decided was
the trajectory: every part started at its angle and its options were
listed from it, so the annealer drew other turns and ties broke the other
way. coco2's gap was that chaos: on x1112 the same imported board (only
the angles changed, `experiments/orientation-search/set_angles.py`) gave
663 vias from 0 degrees and 1060 from the designer's angles, the reverse
of x1100's 1144 and 549.

**Change** (`orientation_search`, placer option, on; `core::orient`):
angle options in one order (0, 90, 180, 270; an odd-angled part keeps its
extra ones), every movable part that may turn starts at the first, turned
about its body centre; fixed parts, parts with one option (`rotation`,
`opening_outwards`, overhangs) and row leaders keep theirs. The lowering
sorts the options too, so the layout's moves walk them in that order. No
mirror: the placer never changes a part's side (no constraint lets it).
Tests `crates/pcb-placer/tests/orientation.rs`: the order, the canonical
start (movable parts only), the placement identical from two sets of
starting angles and from the same start twice (and different with the
option off), a part between two connectors ends turned by a half from
each of four starting angles.

**Stronger searches measured and dropped** (`seeds.py`, placement only,
one seed a run, four seeds, eight boards, against the canonical start):
the annealer's turn proposing the cheapest other angle by wire,
constraints and overlap (coco2 -4 %, homebrew_486 +3 %, OpenFC +4 %, the
rest within 2 %; 15 % more placement work), the global rotation choice
every 5 iterations instead of 15 (geometric mean -0.5 %), a random start
angle per part (+1 %; spell_tome +27 %), rotations from the first
iteration (no change). HPWL from the designer's angles against the
canonical start over 12 boards: geometric mean about 1 %, per board -19 %
(spell_tome) to +16 % (OpenFC, the one board whose designer angles beat
chance: its global placement is 20 % shorter from them); the seeds of
one board spread 10-20 %. The order of the footprints in the file
decides as much as the angles (homebrew_486 sorted by pcbnew's save: 103
of 130 parts elsewhere), and the router follows the order of a
footprint's graphics.

**Per board** (layouts at a third, four threads, one run a side; vias,
KiCad unconnected, weighted HPWL after placement; x1112 / x1130):

| Board | from 0 degrees | from the designer's angles | Placements |
| --- | --- | --- | --- |
| coco2 | 663, 0, 12,414 / the same | 1060, 0, 13,556 / 663, 0, 12,414 | x1130 identical from either start |
| urchin | 41, 0, 1669 / the same | 40, 0, 1692 / 41, 0, 1669 | x1130 identical |
| homebrew_486 | 981, 0, 11,959 / the same | 1196, 0, 13,494 / 1040, 1, 11,959 | x1130 placer poses identical; the race's router probes differ on the input files' graphics order (213 against 183 unfinished for seed 2), so another seed is kept |

**Short tier** x1130 against `build/short-x976.log` (`--jobs 6 --threads
4`, under the lock): every placement changed (the stack's angles no
longer count), passes 6 to 6, unconnected 35 to 32, vias 1129 to 1130,
length 22,253 to 20,777 mm, work 3236 to 2840 s, cut 0. Vias (HPWL from
x1111's placements, whose rows are x976's): USB_Cable_Tester 131 to 90
(1270 to 1279), OpenFC 178 to 154 (756 to 937), framework_mobo 108 to 91
(921 to 850), MIDAS-MK1.1 207 to 198 (2523 to 2137; unconnected 20 to
13), Blue_Line 26 to 25, 0xCB-1337 76 to 78, reCamera 56 to 68 (199 to
190; unconnected 5 to 9), USB_Keypad 78 to 106 (822 to 804), WLED_Matrix
165 to 216 (1079 to 1193); urchin, spell_tome, eternal-keypad the same
vias, other placements. A reshuffle, no cost to the layout tier.
**Schematic tier** x1130 against `build/sch-x1100.log`: 8/12 both, vias
3577 to 3578, ten rows identical; OpenFC (166 to 165) and 0xcb-static (2
to 4) move one part each in the layout's moves (x1112 places them
identically; the moves walk the options in the canonical order).

**Recommendation:** keep it on: one placement whatever the stack holds,
no cost on either tier. It does not make the import's placements better;
nothing measured here beats the seeds' noise, and OpenFC shows designer
angles can carry information the HPWL search does not find. Next, if
anyone takes it up: seed the race's starts with orientations of its own
(half the seeds canonical, half a seeded per-part choice) and judge on
seed-repeated layouts, not single runs; make the placement independent
of the footprints' order too.

Binaries x1130 (the branch on 8216bc9, measured), x1131 (rebased on
8e8b4ba). `cargo test --release -p pcb-kicad -p pcb-placer`,
`agent_walkthrough` and `cli_exit_codes` pass. Logs build/short-x1130.log,
build/sch-x1130.log; runs in build/orient-search/ (`out-*` layouts,
`seeds1`-`seeds3` placements and their tables `seeds*.txt`).

## Committed on 2026-10-15 (starter constraints for key matrices)

Open problem 8, item 4: the import recognises a key matrix from the
netlist alone and writes starter constraints, so that an agent does not
write a keyboard's hundred entries by hand.

**Inference** (`crates/pcb-kicad/src/key_matrix.rs`, `infer_key_matrix`):
- switches: footprint or symbol named `SW`, `KEY`, `MX`, `Choc`,
  `hotswap`, not of a reference class that is never a key (`D`, `LED`, `J`,
  `U`, ...: a USB receptacle `G-SWITCH`, slab's `LED_MX` key LEDs); then
  two-pin parts of any name on a row and a column (capacitive pads);
- a switch's terminals: each of its nets that is no supply (`GND`, `+5V`,
  a `power_in` pin), followed through a diode when the net between holds
  only that switch and one two-pin `D` part (not an LED): the key's own
  diode. A diode net several switches share is followed only when their
  other lines are the same (alternate footprints of one key: le_chiffre's
  1.25u and 2u on one diode), not for a column diode five keys share
  (CyberKeeb2040);
- lines: terminals two switches or more reach; a key alone on its column
  counts when the controller (a part with pins on two lines) has its other
  net (bumwings' mod key);
- rows and columns: the lines two-coloured by the switches between them,
  per connected component; rows by name (`row`, `r2` against `col`,
  `c3`), else the side with fewer nets (jiran-ble-lite's `/P0.07` pins);
  order by the number in the name where every line is named as a row or
  column and has its own number, else by the first switch's reference
  (bumwings' `col_Lp2`, `col_Lp1`, ...; a controller's pin numbers are no
  order);
- halves: two components (urchin, ULK, jiran: separate controllers), or
  one matrix whose column names say the hand (`col_L*`, `col_R*`:
  bumwings); right by name (`R_`, `_r`, `right`); a controller per half
  (most pins on its lines; one shared by both halves is neither's).

**The file** (`starter_constraints`, `constraints.starter.json`, rewritten
on every import, never `constraints.json`): the import's own starter plus
a `row` per matrix row in column order at the key pitch (Choc/PG1350 18 x
17, else 19.05, raised to a unit key's body), consecutive whole rows over
the same columns merged into a grid (`columns`, `row_pitch_mm`: the first
cut with one `row` per matrix row and a `relative` chain missed `relative
SW6 below SW1` by 16 mm on urchin, the rows stacked out of order; the grid
keeps every one), keys larger than the pitch out of the rows and `near`
their neighbour (the keys after them `relative` right of the keys
before), a row with a gap in its columns in pieces (le_chiffre's thumbs
on both hands), each row block `relative` below the one before (thumb
rows, half the longest row or less, up to 10 mm), each diode `near` its
key at 3 mm, and per half a `region` with its rows' first keys and its
controller. `comment` says what was inferred and what needs a decision
(pitch and stagger, thumbs, the controller's edge, the outline, the
switches outside). The import report's `starter`: `matrix` (row and
column nets), `switches`, `diodes`, `halves`, `halves_by`, `rows_by`,
`pitch_mm`, `unassigned`, `wide`, `alternates`, or `why` no file.

Format additions (docs/constraints.md): `region` takes `at` (`left-half`,
`right-half`, `top-half`, `bottom-half`, `center`) in place of `x`/`y`,
cut from the outline's box whatever its size (the starter's outline is
`{}`); a top-level `comment` is accepted and not read.

**Judge** (`experiments/key-matrix/judge.py`, x1121, imports as the
schematic tier does with `--footprints-from` the designer's board, then
checks the starter against the designer's board: a row's switch centres
within one key pitch of their best-fit line, in the row's order, each
`relative` below holding, each diode within its 3 mm):

| board | matrix | switches | diodes | halves | unassigned | rows on a line | in order | below | diodes within 3 mm |
| --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: |
| urchin | 8 x 10 | 34 | 34 | 2 (separate) | 4 | 8/8 | 8/8 | 2/2 | 33/34 |
| le_chiffre 36keys | 5 x 10 | 41 | 35 | 1 | 2 | 5/5 | 5/5 | 2/3 | 35/35 |
| le_chiffre hotswap | 5 x 10 | 36 | 34 | 1 | 1 | 3/3 | 3/3 | 1/2 | 34/34 |
| bumwings v001 | 7 x 14 | 54 | 52 | 2 (column names) | 0 | 11/11 | 11/11 | 6/7 | 52/52 |
| slab-pcb-left | - | - | - | - | 31 | | | | |
| interchange-pcb-right | - | - | - | - | 31 | | | | |
| keyboard (byrantech) | 6 x 14 | 78 | 78 | 1 | 1 | 6/6 | 6/6 | 3/3 | 78/78 |
| ErgoSNM | 5 x 7 | 35 | 35 | 1 | 6 | 2/5 | 4/5 | 0/0 | 11/35 |
| CyberKeeb2040 | 5 x 14 | 67 | 67 | 1 | 2 | 6/8 | 7/8 | 3/3 | 0/67 |
| eternal-keypad | 5 x 8 | 36 | 36 | 1 | 2 | 5/5 | 5/5 | 2/2 | 35/36 |
| ULK | 8 x 12 | 42 | 42 | 2 (separate) | 4 | 8/8 | 8/8 | 2/2 | 42/42 |
| jiran-ble-lite | 10 x 12 | 56 | 56 | 2 (separate) | 4 | 10/10 | 10/10 | 2/2 | 56/56 |
| capybully (capacitive, no diodes) | 4 x 12 | 48 | 0 | 1 | 1 | 5/6 | 6/6 | 1/1 | - |
| Quanta75 | 6 x 15 | 82 | 82 | 1 | 3 | 7/7 | 7/7 | 5/5 | 82/82 |
| poly_corne left | 4 x 6 | 21 | 21 | 1 | 2 | 4/4 | 4/4 | 1/1 | 21/21 |
| osprey | 7 x 6 | 42 | 42 | 1 | 2 | 7/7 | 7/7 | - | 42/42 |
| ghoul, USB_Keypad | - | - | - | - | 43, 9 | | | | |

The matrices are right wherever the board has one; what misses is
geometry the netlist cannot say. slab-pcb (both halves), ghoul and
USB_Keypad wire every key to its own pin (slab and ghoul through an IO
expander or shift register): no matrix, `why` says so, no file. ErgoSNM's
rows curve (up to 29 mm off a line) and its diodes sit 3.3-4.2 mm from
the key; CyberKeeb's diodes are gathered away from the keys (62 mm median)
and two of its rows are bent; capybully's thumb row is bent. Over all
boards but CyberKeeb, 521 of 547 diodes are within 3 mm of their key and
546 within 5 (3 kept, as the walkthrough had). In all, 14 of the 18
keyboards have a matrix; 87 of 93 row lines lie within a key pitch of a
line and 91 come in the row's order. urchin's left matrix is
the designer's right half (`left_is_left` false: the halves are mirrored
on the panel); the regions are a suggestion. Unassigned are the reset,
boot and power switches, an encoder, the TRRS jacks ErgoSNM names `SW`.

**Layout** (urchin, the walkthrough's fixture, the starter alone plus the
walkthrough's 230 x 95 mm outline, `"total_seconds": 30`, four threads,
load 22-31; x1121): exit 0, 42 of 42 constraints kept (2 grids, 2 thumb
rows, 34 `near`, 2 `relative`, 2 `region`), 68/68 routed, KiCad 0
unconnected and no new error, 38 vias, 1960 mm, 19.7 s of work. The
walkthrough's 100 hand-written constraints on the same binary: exit 0,
100/100, 68/68, 53 vias, 1903 mm, 7.2 s of work. The starter as written
(automatic outline): exit 0, 186 x 124 mm, 36 vias, 1902 mm.

**Schematic tier** (`--jobs 6 --threads 4`, under the lock, x1120 = the
branch on cc9c4df; the tier does not read the starter file):
`build/sch-x1120.log` against `build/sch-x1100.log`: **8/12 pass both**,
every row's fields identical (routed, vias, length, constraints, parity,
DRC errors and unconnected, `work_seconds` to the last digit) but
timing and memory (651 s wall against 684; CPU 5702 s against 5208) and
the cosmetic count in `drc_line`: KiCad's `lib_footprint_issues` and
`lib_footprint_mismatch` (spell_tome 9 + 2 against 25 + 0; never judged),
which a tier of another branch at the same time (`build/sch-x1095.log`)
shows alike: the library check's environment, not this change. No tier
with the starter applied: the tier's constraints are the designer's
placement moved onto the import (fixed parts at the designer's spot),
which the starter's rows would contradict rather than add to.

Tests: `key_matrix` (8: rows and columns in order with each key's diode,
rows as the fewer lines with pin-named nets, two matrices as halves named
right, one matrix split by column names, keys wired to their own pins,
alternate footprints and a column diode, a two-pin pad and a ground pin,
the starter's grids, thumbs, wide key, regions and its text parsing as
constraints), `placement_constraints`
`a_region_in_words_is_that_part_of_the_outline_and_a_comment_is_read_past`,
`agent_walkthrough` `the_starter_constraints_of_the_key_matrix_lay_out_the_keyboard`
(import report, file, layout from it at 230 x 95: exit 0, 15 s).
`cargo test --release -p pcb-kicad` (351), `agent_walkthrough` and
`cli_exit_codes` pass. Snapshots `build/bin/pcb-maker-x1120` (on cc9c4df,
the tier), `x1121` (rebased on 8216bc9, the judge and the urchin numbers), `x1122`
(rebased on c9fe4de: key_matrix and the starter walkthrough test pass).

Not done: thumb clusters are a straight `row` below the grid (urchin's
are turned 15 degrees on the designer's board); column stagger is not
inferred (a `row` is a straight line); the controller's edge and the USB
connector's are left to the agent; keys wired to their own pins get no
rows (their positions are not in the netlist); a key's LED (slab's
SK6812 per key) is not tied to its key.

## Committed on 2026-10-15 (search core, x1060 / x1062)

Two measurements and one change to the search proper, routes
byte-identical (x1060: the change on 8a1fd2c, what was measured; x1062:
the commit on c9fe4de). Scripts and logs in the worktree `pcb-maker-core`
(`build/scale`, `build/prof`, `build/bench`, `build/pairs`, `build/fpall`).

**Thread scaling (route mode, x1042, v16 placements, tasks v6,
`RAYON_NUM_THREADS` 1 to 16, one run each, `PCB_MAKER_NO_CACHE=1`).**
Every run of a board gave the same board file (md5) and the same
fingerprints (as a multiset: side-by-side probes print in the order they
finish), so the results are thread-independent as designed. Wall seconds
(CPU seconds):

| board | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| Castor | 302 (369) | 305 (381) | 223 (295) | 225 (296) | 239 (296) |
| k30-SBC | 226 (388) | 234 (396) | 229 (395) | 229 (380) | 226 (401) |
| jetson | 1114 (1744) | 1075 (1781) | 976 (1665) | 851 (1510) | 786 (1370) |

There is no speed-up to speak of: what differs follows the machine's load
(1 and 2 threads ran at load 14 to 30, jetson at 16 threads at 5 to 12),
not the threads. The proof is in the routers' own profile lines: the
negotiation runs its nets in batches, and almost all are single-net
batches, one thread by construction, whose seconds fell as much as the
wall did (Castor 3084 single batches, 132 s at 1 thread, 96 s at 16; the
561 parallel batches with 1214 nets took 6.5 s and 3.2 s of wall; jetson's
first attempt 3617 single batches 209 s to 148 s, 520 parallel 18 s to
9 s). Resolve, clean-up and the via reduction (its negotiations: single
batches again) do not scale either; KiCad's DRC is its own process. What
runs side by side are the probes (four) and the ladder's rungs (three),
std threads sized by free memory, not by `RAYON_NUM_THREADS`: so a
1-thread run still uses up to four cores while it probes (jetson: 1744 s
of CPU in 1114 s of wall). For development jobs: `--threads 1` (or 2) per
board and more jobs gives the same rows for fewer cores; 8 or 16 threads
buy nothing.

**Profile at one thread** (x1042's code with line tables, gperftools
`libprofiler.so`, 200 Hz, route mode, share of the process's samples,
Castor / jetson): the search proper 76 / 87 (`Router::search` and its
heuristic closure). Its lines:

| what | Castor | jetson |
|---|---|---|
| `KeyHeap::pop` (the sift-down's least-of-four loop and compare) | 11.2 | 14.9 |
| `occupancy_seen`: the u16 occupancy read and the own-stamp test | 8.6 | 10.3 |
| search node reads: neighbour closed, via target closed, cost compare, popped node closed | 9.5 | 12.2 |
| the neighbour's `Hot` record (trace owner, history, guard, cover) | 4.7 | 6.0 |
| heuristic closure (`points`: up to 12 target points a push) | 5.5 | 5.1 |
| bounds checks, unattributed lines | 6.8 | 7.2 |
| edge block and owner of the expanded node | 2.4 | 3.4 |
| the via step's per-layer pour cover loop | 2.4 | 0.5 |
| corridor and window tests | 1.7 | 2.5 |
| via allowed, free layers, own via ring | 2.4 | 2.0 |
| heap pushes | 1.3 | 1.4 |
| neck zone test (`zone_class`) | 0.8 | 0.3 |

Outside the search: `Router::stamp` 9.3 / 2.4 (Castor stamps for six
classes), `route_net` with the planner inlined 2.2 / 4.9, the planner's
entry cost 0.9 / 2.0, pour building 2.7 / 0.1. The layout: a
`SearchNode` is 12 bytes (cost f32, three 16-bit generation marks seen,
closed, target, parent u8) per state, states layer by layer (`layer *
cells + cell`, which is also the key's tie-break); occupancy is a dense
`u16` count per map (`Vec<Vec<u16>>`: per class a trace map per layer
and a via map, two diagonal maps per class and layer), the routed net's
own stamps subtracted through a bitset (`own_mark`) only where the count
is not zero; `Hot` is a 12-byte record per (class, layer, cell); the heap
is a four-way heap of one-word keys (estimate bits above the state).
cachegrind of a 3-iteration bench on Castor puts the search's cost in
branch mispredictions (the search's lines: 142 M of 198 M; the neighbour
closed test 21 M, the seen-or-cheaper test 11 M, the heap's sift 25 M, what
the branchless sift removed) and in L1 misses (the node records, the per-layer cover maps).

**Changed** (crates/pcb-router/src/router.rs; the expansions, their
order, every key and cost unchanged). Measured with bench-search
(`PCB_ROUTER_NO_WALL_GUARD=1`, one thread) as two processes on the two
hyperthreads of one core, sides swapped each repetition, user seconds,
median of the ratios (a null test of x1042 against itself: 1.002, spread
0.999-1.003; the machine's load otherwise made one-thread timings swing
by 15 %). Ratios against x1042, Castor (4 iterations) / jetson
(2 iterations), the whole process including the lattice build:

- The key heap keeps its four children per node as an aligned group (half
  a cache line, root at slot 3, slots past the end hold `u64::MAX`) and
  takes the least of the four without branches: 0.996 / 0.990.
- Per expansion, not per step: whether all eight neighbours lie inside the
  window, the corridor's answer when they all lie in the node's tile (77 %
  of nodes; a node outside the corridor there skips its steps),
  the neck zones' bounding box (a step asks the zones only near it); per
  cell, the via step reads one summary word of the pour cover (one net on
  a mask of layers, multiplied in the same layer order; several nets fall
  back to the per-layer maps) instead of a map a layer: together with the
  heap 0.980 / 0.979.
- The via step's target layers as a bit mask (the test against the own
  layer mispredicted) and the unrolled eight steps (one copy a direction:
  the diagonal test and the step's constants are compile-time): all
  together 0.963 / 0.957.

cachegrind of the 3-iteration Castor bench, x1042 to x1060: instructions
31.3 G to 28.1 G, mispredictions 203 M to 147 M, L1 data misses 197 M to
177 M. Tried and dropped (measured as above): a radix heap (slower: keys
are unique, so every pop split a bucket, 438 M key moves for 47 M
expansions), an 8-byte search node (cost and one tag word: 1 % slower on
both boards despite 3 % fewer L1 misses), the node array cell by cell
(a via step's layers on one line; no gain in cachegrind), the expanded
layer's occupancy maps looked up once (0.7 % slower), the heuristic over
four points at a time (more instructions: most searches have one to three
points), touching the next heap top's node early (1.000), a front slot
for the least key pushed since the last pop (1.000).

**CPU, pinned simultaneous pairs** (route mode, `--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1`, x1042 against x1060, which is this change on
8a1fd2c; `cpu_seconds` of the rows; r1 x1042 on 0-7,16-23, r2 swapped):

| board | r1 x1042 / x1060 | r2 x1060 / x1042 (swapped) | both |
|---|---|---|---|
| Castor | 0.904 (299.7 / 270.8) | 0.951 (356.2 / 338.7) | 0.927 |
| k30-SBC | 0.924 (402.3 / 371.7) | 0.958 (392.3 / 375.8) | 0.941 |
| Sisu | 0.932 (1369.4 / 1276.5) | 0.929 (1352.9 / 1256.2) | 0.930 |
| jetson | 0.908 (1432.9 / 1301.5) | 0.961 (1649.8 / 1585.5) | 0.934 |
| spell_tome | 1.012 (17.2 / 17.4) | 1.079 (20.3 / 21.9) | 1.045 |
| geomean | 0.935 | 0.974 | 0.954 (big four 0.933) |

Ratios are new / old; the half a side ran on moved them by 3 to 5 points,
so the swapped pair matters. spell_tome is 1.1 s of work in 17-22 s of
CPU (KiCad's DRC). Every pair: the same board file (md5), vias, work
seconds, verdict and fingerprints (`PCB_ROUTER_FINGERPRINT=1`, as a
multiset). With `PCB_ROUTER_FINGERPRINT_ALL` (every occupancy map and pour
mask hashed into each fingerprint) at a quarter of the budget, two
threads: urchin, spell_tome, Castor, k30-SBC, Sisu and jetson identical to
x1042 (`build/fpall/fpall.out`).

**The work counter** keeps its meaning: the same expansions (work seconds
identical in every row). An expansion now costs about 5-7 % less CPU on
the big boards, so `EXPANSIONS_PER_SECOND` (2.0e6, the idle eight-core
calibration) is stale by about that much. One-thread rates on the loaded
machine (bench-search, 47 M expansions on Castor, simultaneous runs on two
cores) swung between 1.4 M and 1.9 M expansions a second for both
binaries, too loaded to recalibrate from. A recalibration to about
2.1e6 would give every budget 5-7 % more expansions for the same seconds:
every result would move (more search per budget, the same wall per
budget). Not done here.

Short tier (tasks v6, 2 jobs x 4 threads, under the lock) on x1060
against `build/short-x1041.log`: all 12 rows identical but for CPU, wall,
load and memory (routed, vias, length, work seconds, KiCad's findings,
first-route open terminals); compare.py: same 12, unconnected 35 to 35,
passes 6 to 6, cut 0. Its CPU (2756 to 3510 s) is not a claim: unpaired,
and it ran beside the fingerprint runs and other agents (load about 20).
`cargo test --release -p pcb-router -p pcb-kicad` passes (two heap tests
new: pops as a binary heap's, across clears and with keys below the last
popped).

Next on this path: `Router::stamp` (9 % of Castor at one thread, 15 % of
the bench; the footprint loop tests bounds and the mark per offset; most
offsets of consecutive nodes are already marked), and the mispredicted
data-dependent tests that remain (closed neighbour, seen-or-cheaper,
occupancy zero), which no layout change removes.

## Committed on 2026-10-15 (mask openings block placement)

The open item of the mask-bridge section: the placer let parts sit under
a fixed part's mask opening. Cause: the body's box holds a footprint's
mask graphics (`body_on_side`), but a part whose courtyard covers under
three quarters of that box blocks with its courtyard's shape and pads
(`hollow`), without them. SNSP-CPU-01's J2 (RFModulator: 12 x 5 mm
courtyard, 22 x 32 mm F.Mask polygon) and SNSP-CPU-1CHIP's RF1 are such
parts; `PCB_PLACER_DESIGNER` on the v16 layouts judged T1 and C61 (and
1CHIP's T1) legal under the opening on x1055, illegal ("shaped or hollow
boxes", J2) on x1110. Fix (crates/pcb-kicad/src/board_placer.rs,
`mask_openings`, on): the shape includes the part's mask graphics on its
side, held to the body's box (a stroke's half width past it would raise
`far_reach` on every search), so the legality test and the annealer keep
bodies and pads off it, fixed and movable parts alike. Where KiCad's mask
margin, max(web + `pad_to_mask_clearance`, `solder_mask_to_copper_clearance`
(project_rules, new placer key `solder_mask_to_copper_mm`)), exceeds the
copper clearance, the openings grow by the difference (eternal-keypad,
protoesp, USB_Cable_Tester, WLED_Matrix, USB_Keypad: none has such a
part). Same-net pads are kept out too. The designers' placements of both
SNSP boards: the same parts illegal before and after (51 and 47, none
for an opening).

Census (`experiments/mask-openings/pads_under_openings.py`: other
footprints' pads whose box meets a footprint's mask graphics, per side):
0 on all 109 designers' boards; on the 109 v16 layouts only SNSP-CPU-01
(J2: T1 4 pads, C61 2) and SNSP-CPU-1CHIP (RF1: T1 4 pads). Parts with a
courtyard and mask graphics past it: 7 on 3 boards (SNSP-CPU-01 4,
1CHIP 2, USB_Cable_Tester's battery holder 1).

Layouts, tasks v6, one run a side, jobs 1, threads 4 (x1055 = 8136361,
x1110 = that + this; build/openings-snsp-x1055, -x1110):

| board | pads under openings | solder_mask_bridge | unconnected | errors | vias | first route |
| --- | --- | --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 6 / 0 | 15 / 0 | 102 / 147 | 16 / 2 (copper_edge_clearance 1 / 2) | 1812 / 1615 | 114 / 215 |
| SNSP-CPU-1CHIP | 4 / 0 | 16 / 0 | 2 / 0, passes | 16 / 0 | 757 / 678 | 9 / 6 |

SNSP-CPU-01's unconnected rose: front utilization 0.732 to 0.757 (J2's
opening), the race kept seed 3 instead of 12 (probe 308 unfinished nets
against 317, wirelength 14235 to 14556 mm), its first route left 215
open terminals against 114. One run cannot tell a reshuffle of this
crowded board from a cost of the rule (v16, x862, with the parts under
J2: 152); a few placer seeds per side would.

Identity: place-kicad-board (short-x1054's prepared sources and
layout.json, 120 s of placement work, four threads), x1055 against
x1110: byte-identical boards on spell_tome_bottom, USB_Cable_Tester,
urchin, eternal-keypad, OpenFC and Blue_Line (board-placer.json differs
only in seconds and paths). Short tier on x1111 (x1110 rebased on
2ab5ef5; tasks v6, jobs 6, threads 4, under the lock,
build/short-x1111.log) against build/short-x976.log: compare.py same
12, unconnected 35, new errors 3, vias 1129, length 22253 mm, work 3236
s, first-route 61 on both; no row field differs but `tasks` (the path's
spelling) and timing; every placed and result board byte-identical to
build/short-x1054's. Unit tests
`a_fixed_parts_mask_opening_keeps_other_parts_pads_out` (J2's geometry
at its angle, J2 fixed and movable, legality and the annealer's overlap,
the switch off) and
`a_mask_opening_keeps_kicads_mask_margin_beyond_the_copper_clearance`.
Not done: a pad's own mask expansion beyond the spacing, and the grown
margin for whole-body parts (their box holds the openings at the
spacing only).

## Committed on 2026-10-15 (schematic tier failures)

Triage of the seven failing rows of the schematic tier's x1071
(`build/sch-x1071.log`). Binaries: x1100 (the branch on f1698bf, measured),
x1101 (rebased on 8136361, main's x1055), x1102 (rebased on 2ab5ef5,
main's x1093); no tier on either. Runs, one per
side: the boards at the full budget (`--budget-scale 1`) on x1072 (main's
code: MokyaLora, slab-pcb-left) and x1100 (slab-pcb-left), the layout tier
at a third on x1100 (tasks v7: coco2, OpenFC, TelemetryOnboard;
`build/schfail-layout-third-x1100.log`), the layout tier's v16 and x903
rows.

| Board (x1071) | Mechanism | Specific to the schematic flow? | Done |
| --- | --- | --- | --- |
| urchin, copper_edge_clearance 3 | The designer's `.kicad_pro` excludes 5 pads over the edge (`copper_edge_clearance\|x\|y\|<edge id>\|<pad id>`); the tier drew the outline with ids of its own and the import gave the board's footprints new pad ids, so 3 of the 5 came back as new errors (the designer's run counts 2, the other 3 being excluded). Our edge floor and the outline's position were not it: rules (0 mm) and geometry identical. | Yes (ids) | Fixed: passes |
| le_chiffre, copper_edge_clearance 1 | The same: one excluded pad, its KiCad 7 pad `tstamp` kept by accident (the renewal only knew `uuid`), the outline's id new. | Yes | Fixed: passes |
| MokyaLora, KiCad 1 unconnected | Mtr2's shield pad (a custom pad with primitives, GND, on the back): the router never reaches it (261/262 in the layout tier too), KiCad joins it through the designer's GND pour on six layers, which the tier did not copy (it got the router's automatic In1/In4 planes). At the full budget without it (x1072): still open. | Yes (zones) | Fixed: passes |
| MokyaLora, courtyards_overlap 1 | R34 (0 ohm) and R35 (DNP) at the same spot by design, excluded by footprint ids in the project. | Yes (ids) | Fixed |
| slab-pcb-left, KiCad 2 | The budget: at the full budget it passes without the pours (x1072, 76/76, 718 s of work) and with them (x1100, 76/76, 332 vias; the layout tier v16 315). | No | Budget (x1100 at a third: 75/76, KiCad 1) |
| USB_Keypad, KiCad 2 | The layout tier's own loss at every budget: a decoupling capacitor at -1.9, 12.7 (C11 in v16, C15 in x903 and x1100), its GND pad off the hatched pour, its 3.3 V pad short of its track. | No | - |
| OpenFC, KiCad 2 | U10 (SOT-23-5 on the back): v16 81/82 at the full budget (U10 pad 5), short-x976 81/82 KiCad 2, the layout tier at a third on x1100 81/82 KiCad 2; with the designer's pours a GND island on B.Cu (zone to zone) and U10's pad 5. | No | - |
| TelemetryOnboard, KiCad 2 | The routing at the budget: the USB-C pair and U1's pins; the layout tier at a third 107/108 KiCad 2, v16 at the full budget 107/108 KiCad 1 (J1's D+ pair). | No | - |

Candidates ruled out: the generated `fixed`/`place`, `edge`, `back`/`front`
and `add` entries (the failing parts were at the designer's spots and
sides, footprints identical to the designer's; the drawn outline identical
shape for shape on urchin), drift (none on these boards but OpenFC's four,
which cost nothing since x1071).

**Fixes:**
- *The designer's DRC exclusions apply* (`netlist_import.rs`,
  `kept_uuid`): with `--footprints-from`, a symbol's own footprint (and a
  `from_board` `add`) keeps the board footprint's uuid and its pads' and
  drawings' ids, the first time that board footprint is used (a copy of
  it as a template for another part gets new ones), as KiCad's "Update
  PCB from Schematic" keeps them. Library footprints renew `tstamp` ids
  too (KiCad 6 and 7 files): every instance of one had the same ids.
  `schematic.edge_cuts` keeps each Edge.Cuts shape's id (`uuid` or
  `tstamp`). docs/agents.md says so. Test
  `footprints_from_the_board_keep_their_ids_for_the_drc_exclusions`
  (the board's ids kept for the own footprint and the add, new ones for a
  copy, the KiCad 6 library's tstamps renewed, no id twice; fails on the
  old code twice over).
- *The designer's zones* (`schematic.zones`, run.py): the designer's board
  through `strip-kicad-tracks` (what the layout tier keeps: rule areas and
  the pours covering a good part of the board, unfilled), its zones in
  KiCad 10's syntax (`(net "GND")`, `uuid`) on the imported board's names
  of their nets (`designer_matches`, shared with `additions`). Rows have
  `zones` (1 to 43 per board). A tier-generation gap: every one of the 12
  boards has pours (2-layer boards were routed without any, 4- and
  6-layer ones with the router's automatic planes) and 4 have rule areas
  (TelemetryOnboard 36, MokyaLora 10). All 12 generated boards load in
  KiCad (kicad-cli DRC exit 0, every pour's net on the board).
- *`stopped` follows the pass rule*: `routing` only on KiCad's unconnected
  items (MokyaLora passed at 261/262 with KiCad at 0 but said `routing`).
- Tests `benchmarks/agent-tasks/test_schematic.py` (outline ids, zones in
  KiCad 10's syntax on the imported nets, rule areas without a net, a
  net no pad shows keeps its name, KiCad 10 designer zones).

**Schematic tier** x1100 against `build/sch-x1071.log`, 6 jobs x 4 threads,
under the lock, load 12-32: **5/12 to 8/12** (684 s wall against 1168),
unconnected 9 to 8, new errors 5 to 0, vias 2877 to 3577, work 2247 to
3710 s, cut 0. Passes: urchin and le_chiffre (exclusions), MokyaLora (the
pour and the exclusion). Every row but spell_tome's changed, each
explained in the tier README ("x1100"): the pours add vias (coco2 661 to
1144, homebrew_486 907 to 1008, USB_Keypad 94 to 110) and work (the router
analyses pours; slab-pcb-left 74/76 to 75/76, TelemetryOnboard KiCad 2 to
3, OpenFC 80/82 to 81/82).

coco2's vias against the layout tier at a third (573 vias, 22,373 mm
against 1144 and 26,093): not the pours (GNDS vias 177 against 140) but
the orientations. The same imported coco2 board laid out again by hand
(x1100, a third, 4 threads) with every movable part stacked at the
layout tier's point gives the tier's row exactly (1144, 26,093); with the
designer's angles set on the parts (pads turned with them) 549 vias and
22,505 mm. The layout tier's stack keeps the designer's orientations, the
import puts every part at 0 degrees, and the placement follows the
orientations it starts from. Open problem 9, item 8.

**Short tier** (tasks v6, `--threads 4 --jobs 2`, under the lock) x1100
against `build/short-x976.log`: 12 of 12 rows identical (routed, vias,
length, work, KiCad; unconnected 35, new errors 3, 1129 vias, passes 6,
work 3236 s, cut 0). The import and the
schematic tier's scripts are not on the layout tier's path.

`cargo test --release -p pcb-kicad` (342 on x1102), `agent_walkthrough` and
`cli_exit_codes` pass. Logs: build/sch-x1100.log, build/schfail-full-x1072.log,
build/schfail-full-x1100.log, build/schfail-four-x1100.log (the first
four boards), build/schfail-layout-third-x1100.log, build/short-x1100.log.

## Committed on 2026-10-15 (solder mask bridges, x1050-x1054)

The premise (our vias bridge pads of other nets) does not hold: no via-pad
or via-via solder_mask_bridge in any of our outputs. Census
(`experiments/pour-fidelity/mask_bridges_census.py`: findings of a run's
drc.json by item pair against the designer's own board through
`drc-kicad-board`, with the values KiCad's rule reads from pcbnew) over
build/github-layout-v16, short-x1020, quick30-x903 and seams-x910: boards
with more bridges than the designer, or a pair kind the designer has
fewer of (build/mask-census-before.json):

| run | board | ours | designer | beyond the designer, by pair | expansion | web | mask-cu | tented |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| v16 | OpenESC 4in1-mini | 52 | 33 | opening-track 13, opening-via 5, opening-zone 7, track-pad 2 | 0 | 0 | 0.005 | both |
| v16 | OpenESC 4in1 | 8 | 0 | track-pad 7, pad-pad 1 | 0 | 0 | 0.005 | both |
| v16 | eurorack-pmod | 1 | 2 | opening-track 1 | 0 | 0 | 0 | both |
| v16 | SNSP-CPU-01 | 20 | 5 | opening-pad 16, opening-zone 4 | 0 | 0 | 0 | both |
| v16 | SNSP-CPU-1CHIP | 16 | 0 | opening-pad 8, opening-track 4, opening-zone 4 | 0 | 0 | 0 | both |
| v16 | protoesp, laptop motherboard | 199 | 199 | pad-pad same part 2, 5 (DRC limit) | | | | |
| x910 route | OpenESC 4in1-mini | 114 | 33 | opening-pad 45, opening-track 12, opening-via 15, opening-zone 6, track-pad 6 | | | | |
| x910 route | SNSP-CPU-01 | 18 | 5 | opening-pad 15, opening-zone 3 | | | | |

No bridge beyond the designer's on the short and quick tiers. Every board
of the corpus tents its vias but SUMEC (`viasonmask yes`, with expansion,
web and mask-to-copper all 0: its vias would need to touch), so a
via-to-via mask spacing binds nowhere and was not built.

KiCad's rule (pcbnew/drc/drc_test_provider_solder_mask.cpp, 10.0.7): an
item on a mask layer (pad with that layer, untented via, track on a mask
layer, mask graphic or text) against another: shapes closer than
`solder_mask_min_width` plus both expansions; covered copper (track,
tented via, a pad without the mask layer) against an opening: closer than
`solder_mask_to_copper_clearance` plus the opening's expansion; zone fill
likewise. Same-net pairs never; a plain opening (graphic, mask-only pad)
once it exposes two nets. Expansion: pad, footprint, else the board's
`pad_to_mask_clearance`; vias the board's; texts none. KiCad draws an
outline-font text from the font, a missing face in a stand-in, not from
the render cache.

Causes, by pair: opening-pad is placement (SNSP's C63, C64, C66 or T1
and C61 placed under the fixed J2's 22 x 32 mm F.Mask polygon; 1CHIP's T1
under RF1's; the placer's report puts them there, so its mask-as-body
rule does not keep movable parts out of a fixed part's opening, for a
placement agent); opening-zone follows from it (the GND fill the designer
also has under J2 meets a second net only because the pads are there);
1CHIP's opening-track is T1's own stubs. Router causes: (1) a pad's
aperture kept the margin alone, without the project's
solder_mask_to_copper_clearance (OpenESC 4in1's J1: margin 0.102, plus
0.005; our tracks kept 0.105); the latent `solder_mask_margin` read meant
the board's expansion never counted. (2) Mask texts: 4in1-mini's "ESC"
in Tokyo, drawn by KiCad in a stand-in 2 mm wider than the cache we
routed around; eurorack's "[apf.audio]" (stroke font, bottom-anchored:
the "p" hangs 0.25 mm past the anchor, our box stopped at it), and after
that fix its "⇒⊕⊥∃⊢≢" (symbols 0.73 mm across the line of a 1 mm text).

Fix (crates/pcb-kicad/src/board_router.rs): pad apertures per mask side
keep covered copper at max(old margin rule, mask-to-copper + expansion)
and untented vias at web + pad expansion + via expansion; the board's
expansion is `pad_to_mask_clearance`; only pads with the mask layer open
it; a reach equal to the copper clearance adds no obstacle (it rerouted
USB_Cable_Tester for nothing). `vias_tented` reads KiCad 10's, KiCad 9's
and the old plot form. A cached text on a mask layer also opens the
stroke box (`mask_graphic_shapes`; the box holds all nine cached mask
texts of the corpus within 0.06 mm of KiCad's drawing); the stroke model
hangs descenders below a bottom anchor and counts non-ASCII as tall.
All of it only where solder_mask_bridge is an error (KiCad's default; 95
of 109 projects): on OpenESC (warning) the exact rule took 4in1-mini's
bridges 29 to 2 and cost 52 to 61 unconnected (4in1 104 to 106), the
min-connection precedent (`solder_mask_bridge_severity`, project_rules).

Route mode (tasks v6, jobs 1, threads 4), x978 (main) against x1054:

| board (placement) | x978 unconnected, bridges | x1054 |
| --- | --- | --- |
| SNSP-CPU-01 (v15) | 191, 18 (13 new) | identical board |
| SNSP-CPU-01 (v16) | 152, 20 | identical board |
| SNSP-CPU-1CHIP (v16) | 2, 16 | identical board |
| OpenESC 4in1-mini (v16, warning) | 52, 29 | identical board |
| OpenESC 4in1 (v16, warning) | 104, 4 | identical board |
| eurorack-pmod (v16) | 18, 1 | 13, 0 (136 vias against 132, 3131 mm against 3075) |

SNSP's 13 cannot go by routing: all 18 findings pair J2's F.Mask polygon
with a pad of C63, C64 or C66 or with the GND fill under it (and the fill
is the designer's too). x1051 (exact rule everywhere) gave the table in
the severity paragraph. Starved-thermal census (fill3 KiCad cache, 44
boards) with x1050 and x1054: 162 / 7 / 26 / 2, unchanged. Short tier
(flock, tasks v6, jobs 2, threads 4) x1054 against build/short-x1020.log:
all 12 rows identical (35 unconnected, 3 new errors, 1129 vias, 22253 mm,
first-route 61); work 2671 to 3236 s is main's accounting (x976's rows
exactly). x1053 (before the equal-reach rule) changed only
USB_Cable_Tester (131 to 105 vias, first route 0 to 1, pass both).
Unit tests: `a_pads_mask_aperture_keeps_the_mask_to_copper_clearance_and_an_untented_via_the_web`,
`at_snsps_values_a_via_by_another_nets_pad_bridges_no_mask`,
`below_an_error_the_pad_apertures_keep_the_rule_of_before`,
`via_tenting_is_read_as_kicad_10_reads_it`,
`a_cached_text_on_a_mask_layer_also_opens_the_stroke_fonts_box`,
`a_bottom_anchored_texts_descenders_hang_below_its_anchor`,
`symbols_outside_ascii_reach_as_far_as_descenders`. Logs
build/mask{,2,3}-*.log, build/short-x1053.log, build/short-x1054.log,
build/mask-census-*.json, build/mask-thermal-census-x1054.txt.

## KiCad check cost (2026-10-15)

The question: how much of an uncached KiCad check (`kicad-cli pcb drc
--format json --severity-all [--schematic-parity] --refill-zones`, 5-35 s,
about 2.2 GB) is fixed cost that a persistent checker would remove, and
can we build one? Measured, not integrated; scripts and logs in
`experiments/kicad-daemon/` (KiCad 10.0.7, four pinned CPUs, load 8-33
from other agents: walls are +-2 s, CPU seconds are steadier).

**Where the fixed cost is.** `kicad-cli version` takes 0.17 s. A DRC of an
empty board takes 3.0 s wall, 9.1 s CPU and 2.1 GB, and the same DRC with
`KICAD_CONFIG_HOME` pointing at empty `fp-lib-table`/`sym-lib-table` takes
0.4 s, 0.4 s CPU and 110 MB. The 2 GB is KiCad 10 loading every footprint
library of the global table (155 libraries, 181 MB of `.kicad_mod`):
`PCBNEW_JOBS_HANDLER::JobExportDrc` calls `FootprintLibAdapter(...)->
AsyncLoad()` and `BlockUntilLoaded()` before anything else, for the library
parity test, whose `lib_footprint_*` warnings pcb-maker never counts
(`is_library_metadata_warning`, `is_cosmetic`). The board size plays no
part. The empty `sym-lib-table` makes no difference to the DRC.

v16 layout results (`measure_cli.py`, board-only projects, so no parity;
best of two runs). The process+init column is the empty-library empty-board
run. The library column is the default call minus the empty-library call.
Refill is the empty-library call minus a run on the board KiCad filled and
saved, without `--refill-zones`. The DRC column is what is left, board load
included (0.07-0.5 s through pcbnew, see below). Writing the report takes
1-100 ms (the time between kicad-cli's `Found` and `Saved` lines):

| board | wall s | CPU s | RSS MB | process+init | library preload | refill | DRC + load | wall / RSS without libraries |
|---|---|---|---|---|---|---|---|---|
| spell_tome | 5.8 | 10.5 | 2155 | 0.4 (7%) | 1.2 (21%) | 0.8 (13%) | 3.4 (59%) | 4.6 s / 245 MB |
| Castor | 5.4 | 13.1 | 2187 | 0.4 (7%) | 3.5 (66%) | 0.7 (12%) | 0.8 (15%) | 1.8 s / 356 MB |
| WLED_Matrix | 18.3 | 29.6 | 2294 | 0.4 (2%) | 2.5 (13%) | 9.8 (54%) | 5.6 (31%) | 15.8 s / 560 MB |
| ghoul | 21.4 | 27.7 | 2228 | 0.4 (2%) | 1.4 (7%) | 3.6 (17%) | 16.0 (74%) | 20.0 s / 413 MB |
| 0xCB panel | 31.9 | 75.0 | 3003 | 0.4 (1%) | -0.5 (noise) | 28.4 (89%) | 3.6 (11%) | 32.4 s / 1695 MB |
| jetson | 8.6 | 15.3 | 2256 | 0.4 (5%) | 3.5 (41%) | 1.3 (15%) | 3.4 (40%) | 5.1 s / 510 MB |
| sum | 91.4 | 171.2 | | 2.4 (3%) | 11.7 (13%) | 44.6 (49%) | 32.8 (36%) | 79.7 s, 134.6 s CPU |

The fixed cost (process, init, library preload) is 15% of the wall over the
six boards and 21% of the CPU (36.6 of 171 s). It is above a third only on
quick checks (Castor 73%, jetson 46%). The rest is per board: the refill
(the panel's 28 s, WLED's 10 s with its teardrops) and the DRC proper
(ghoul's 16 s). A persistent process cannot cache either, because every
check is on a new board. Peak memory falls from 2.2-3.0 GB to 0.25-1.7 GB.
With the empty tables the findings are the same per type and severity on
all six boards, except `lib_footprint_issues` (more: every footprint's
library is now "not in the table") and `lib_footprint_mismatch` (gone),
both warnings. Castor's designer schematic with `--schematic-parity`: 4.5 s
/ 2.3 GB to 2.1-3.5 s / 0.46 GB, the 205 parity entries identical; parity
itself costs 0.5-0.9 s. ERC (`sch erc`, Castor) is 2.7-3.4 s / 1.3 GB
against 1.0 s / 0.3 GB, but its library warnings change
(`lib_symbol_issues` 47 to 248, `lib_symbol_mismatch` 134 to 0,
`footprint_link_issues` 124 to 158); check how ERC warnings are used before
applying the change there.

**Persistent checker options.**
- KiCad IPC API (kipy 0.8.0, API 10.0.6, read from its protobufs): only a
  running GUI KiCad with the API server enabled serves it, and Xvfb is not
  installed. Its board commands have no way to open a board file (only
  Save/Revert of the open document), `RefillZones`, `InjectDrcError` and
  `RunBoardJobExport*` (no DRC job). It cannot run a DRC or return
  findings. Not feasible.
- A pool of warm kicad-cli processes: each invocation checks one board and
  exits; there is no server mode (10.0.7: fp, jobset, pcb, sch, sym,
  version). Not feasible.
- The pcbnew Python module (SWIG): `LoadBoard`, `ZONE_FILLER(board).Fill`,
  `WriteDRCReport`. Standalone, `WriteDRCReport` segfaults in 10.0.7
  (`assert "process" failed in Pgm()`): the library parity test reaches
  `Pgm().GetLibraryManager()`, and standalone Python has no PGM.
  `ClearProject()` gets past that and crashes later. It works once
  `daemon_drc.py` builds what kicad-cli has, through ctypes on exported
  symbols: `wxInitialize` (a console wxApp), the `PGM_BASE` constructor,
  `SetPgm`, `PGM_BASE::InitPgm(headless)`. That is mangled C++ names in a
  1 MB buffer, tied to libkicommon 10.0.7.

**Prototype numbers** (`daemon_drc.py`, all six boards in one process,
`results/daemon-2026-10-15.log`). Imports and bootstrap take 0.3 s. Board
load is 0.06-0.47 s. A second DRC on the loaded board costs the same as the
first (Castor 0.91 s against 0.97 s, ghoul 18.4 s against 18.3 s), so the
engine keeps nothing worth having across runs. Per type and severity the
findings equal kicad-cli's (empty libraries) on five of six boards. WLED
differs (`connection_width` 4 against 57, `copper_sliver` 0 against 24):
kicad-cli's refill is `ZONE_FILLER_TOOL::FillAllZones`, which first
rebuilds the teardrops (`TEARDROP_MANAGER`, not in the SWIG API). On WLED's
board as KiCad saved it, teardrops included, refill and DRC match exactly.
Paired with kicad-cli under empty tables on the same CPUs (`pair.sh`,
`results/pair-2026-10-15.log`, two alternations each), a warm daemon call
against a cold process is: spell 4.5-5.0 against 6.1-7.0 s, Castor
2.2-2.3 against 2.6-2.8, jetson 5.8-8.7 against 5.5-7.9, ghoul 17.0-19.2
against 18.0-21.1, panel 27.0-27.1 against 27.2-27.4. Summed: 59.4 against
62.7 s (5%). Also:
- WriteDRCReport has no schematic parity (it passes `false`).
- Its output is a text report without item UUIDs (pcb-maker reads `uuid`
  for silk_overlap labels and pad positions from `unconnected_items`); the
  DRC items are not exposed and it adds no markers.
- RSS grows from 0.3 to 3.2 GB over the six boards and is not returned.
- The SWIG bindings have been deprecated in favour of the IPC API since KiCad 9.

**Recommendation: no daemon.** After the library fix it would save about
0.4 s per call (5%), lose parity and teardrops, and need a JSON writer that
SWIG cannot feed. Its only robust form is a C++ host built against KiCad's
own source, which is out of proportion. What a persistent process would
have removed is the library preload, and an environment variable removes
it today: run kicad-cli with `KICAD_CONFIG_HOME` set to a pcb-maker-owned
directory (for example `<cache root>/kicad-config/10.0/` with empty
`fp-lib-table` and `sym-lib-table`, created on first use), in
`run_report_command_cached` and `run_kicad_save_board`. This saves 13% of
the wall and 21% of the CPU per uncached check (8.7 CPU s on an empty board),
and 1.5-2 GB of peak memory, which lets more checks and jobs run side by
side. Size: about 60-100 lines in native_report.rs plus a test with the
fake kicad-cli.
- The DRC cache key already hashes `$KICAD_CONFIG_HOME`'s tables. It must
  take the override's, so every entry misses once.
- A project whose `.kicad_pro` sets `lib_footprint_issues` or
  `lib_footprint_mismatch` to `error` would then count one error per
  footprint. None of the 109 v16 projects does (75 warning, 6 ignore for
  issues; 69 and 12 for mismatch). Fall back to the user's configuration
  for such a project.
- A project-local `fp-lib-table` still loads its own libraries. That is the
  designer's choice and usually small.
- `kicad_common.json` is created fresh in the override directory. kicad-cli
  rewrites it anyway, and the cache leaves it out.
- ERC can take the same directory once its changed library warnings are
  shown not to count.

**Built (x1091 on 0121d54; x1092 rebased on 8136361).** `kicad_config.rs`:
kicad-cli's DRC (`run_kicad_report` for `pcb drc`, `run_kicad_save_board`)
runs with `KICAD_CONFIG_HOME=<cache root>/kicad-config`. That directory
holds `10.0/fp-lib-table` and `10.0/sym-lib-table`, both empty. They are
created on first use and rewritten if changed. The root is the DRC cache's
(`PCB_MAKER_CACHE_DIR`, `XDG_CACHE_HOME`, `HOME`) and is used even with
`PCB_MAKER_NO_CACHE=1`. The user's configuration is kept:
- with `PCB_MAKER_KICAD_USER_LIBS=1`;
- when the board's `.kicad_pro` sets a `lib_footprint_*` rule severity to
  `error`, or is not JSON;
- when the directory cannot be prepared;
- for ERC, which stays on the user's configuration until its changed
  library warnings (`lib_symbol_issues`, `lib_symbol_mismatch`,
  `footprint_link_issues`) are shown not to count.

The cache key hashes the tables of the configuration kicad-cli runs with
and the `KICAD_CONFIG_HOME` it sees, so every entry missed once after the
switch. `netlist_import.rs`'s own DRC call is unchanged. Tests:
`kicad_config` (version series, tables written and repaired, a project with
library errors keeps the user's libraries) and a fake-kicad-cli test (the
DRC sees the empty tables through `KICAD_CONFIG_HOME`; the same
configuration hits; the user's configuration is a second entry and a second
run). `cargo test --release -p pcb-kicad` (341) and `-p pcb-maker --test
cli_exit_codes` (7) pass.

Six boards, `pcb-maker drc-kicad-board` with the cache off, user against
override, best of two alternated runs on four pinned CPUs
(`judge_override.py`, `results/judge-x1091-2026-10-15.log`). Findings per
type and severity are identical on all six, except the `lib_footprint_*`
warnings:

| board | wall s | CPU s | peak RSS MB |
|---|---|---|---|
| spell_tome | 6.7 / 4.1 | 12.9 / 3.9 | 2140 / 245 |
| Castor | 4.7 / 2.2 | 11.7 / 2.9 | 2182 / 349 |
| WLED_Matrix | 18.2 / 14.8 | 30.4 / 21.1 | 2300 / 585 |
| ghoul | 20.3 / 17.8 | 26.0 / 18.9 | 2196 / 386 |
| 0xCB panel | 29.2 / 28.4 | 72.8 / 67.4 | 2951 / 1654 |
| jetson | 7.9 / 5.5 | 14.4 / 7.3 | 2275 / 508 |
| sum | 87.0 / 72.8 (-16%) | 168.2 / 121.5 (-28%) | |

x1092 on Castor: 4.6 / 1.9 s, 11.4 / 2.5 CPU s, 2180 / 356 MB, same
findings.

Short tier under the lock (`--jobs 6 --threads 4 --min-free-gb 20`, tasks
v6, load 5-25), x1091 against build/short-x976.log: all 12 rows identical
except timing and memory (unconnected 35, new errors 3, 1129 vias,
22253 mm, 6 passes, work 3236 s, cut 0). Peak RSS per row is now the
layout's own wherever KiCad set it:

| board | peak RSS MB |
|---|---|
| WLED_Matrix | 2326 to 792 |
| USB_Keypad | 2244 to 1202 |
| Blue_Line | 2325 to 2092 (its layout) |
| largest of the tier | 2.3 to 2.0 GB |

CPU is 3258 to 3327 s, but the runs differ: x976 ran on a warm DRC cache
(0.2-0.3 s designer checks) and x1091 on a cold one, since the new keys
missed everywhere (designer checks 1.5-14.8 s). The per-check CPU saving
is the six-board table above. Tier wall 1161 s.
build/short-x1091.log.

**Follow-up: ERC checked, import DRC configured (x1095 on cc9c4df, x1096
rebased on 8216bc9).**

The DRC that `netlist_import.rs` runs for the pad check now takes the
library-free configuration (`kicad_config::configure_drc`, which
`run_kicad_save_board` uses too). It reads only clearance findings.

ERC stays on the user's configuration. I ran `sch erc --format json
--severity-all` on the designer schematics of the twelve schematic-tier
boards and Castor's, user configuration against empty tables
(`judge_erc.py`, `results/erc-2026-10-15.log`):
- Every type is identical except three warnings: `lib_symbol_issues`,
  `lib_symbol_mismatch` and `footprint_link_issues`. They change on all
  thirteen.
- The ERC drops from 2.0-6.3 s / 6-13 CPU s / 1.2-1.5 GB to 0.4-2.7 s /
  0.4-2.7 CPU s / 0.26-0.37 GB.
- The condition "only `lib_*` types differ" fails on
  `footprint_link_issues`.
- More important, `erc_violations` in verification.json counts every ERC
  finding, warnings included, and `complete` needs it to be 0. With empty
  tables the totals rise: spell_tome 0 to 38, TelemetryOnboard 3 to 351,
  homebrew_486 19 to 436. spell_tome's verification would no longer be
  complete.

So moving ERC means first deciding that these three warnings are library
metadata, like `is_library_metadata_warning` for the DRC, and leaving them
out of `erc_violations`. That changes what verification.json counts (it
already depends on the libraries installed: `lib_symbol_mismatch` 240 on
coco2 under the user's configuration). It is not done.

Schematic tier under the lock (`--jobs 6 --threads 4 --min-free-gb 20`,
tasks v7, load 18-42), x1095 against build/sch-x1100.log:
- All 12 rows are identical except time, memory and the DRC cache line.
  8 pass. Objectives are the same: unconnected 8, new errors 0, 3577 vias,
  92070 mm, work 3710 s.
- Parity is identical per row (40 issues: OpenFC 5, TelemetryOnboard 16,
  slab-pcb-left 6, urchin 6, bumwings 7).
- Errors, unconnected items and warnings in every `drc_line` are the same.
  Only the cosmetic counts differ: x1100's reports had the user's
  `lib_footprint_*` warnings.
- Peak RSS: OpenFC 2246 to 1270 MB, USB_Keypad 2292 to 1280, spell_tome
  2185 to 1263, 0xcb-static 2273 to 1360, TelemetryOnboard 2263 to 1975.
  The others are their layout's.
- CPU 5208 to 5024 s.
- x1100 shows the user's library warnings, so most of this saving is the
  DRC override of 2ab5ef5 plus the pad check.

Tests: `cargo test --release -p pcb-kicad` (344) and `-p pcb-maker --test
cli_exit_codes --test agent_walkthrough` (7 and 2) pass.

**ERC without the global libraries (x1098 on c9fe4de).** Decision: ERC's
`lib_symbol_issues`, `lib_symbol_mismatch` and `footprint_link_issues`
warnings are library metadata. They depend on the libraries installed where
KiCad runs: coco2 had 240 `lib_symbol_mismatch` under the user's
configuration, which is wrong for a verdict.
- `is_erc_library_warning` and `erc_design_issues` (native_report.rs,
  exported) keep them out of verification.json's `erc_violations`. They are
  counted by type in the new `erc_library_warnings`, which defaults empty
  when reading older files.
- The same filter applies to the introduced-ERC comparisons in
  sequential_router.rs and pcb-benchmark.
- A library rule that a project rates `error` still counts, and such a
  project keeps the user's libraries (`erc.rule_severities`, as for the
  DRC).
- ERC (`sch erc` through `run_kicad_report`) now runs with the same
  `KICAD_CONFIG_HOME`. `PCB_MAKER_KICAD_USER_LIBS=1` restores the user's.
- The verification cache's policy is now
  `verify-materialized-rung-v2:erc-without-library-warnings:...`, so cached
  verifications with the old count are not reused.
- Before this change, `erc_violations` included these warnings: coco2
  counted 295 where it now counts 0.

Judge (`judge_verification.py`, `results/verification-x1098-2026-10-15.log`):
`verify-kicad-rung` with every cache off on the twelve schematic-tier
results of x1095 and on Castor's designer schematic beside its v16 board,
user configuration against empty tables:
- `erc_violations` is identical on all 13, and so is `complete` (false on
  all 13, for DRC or parity reasons).
- The library warnings appear only in `erc_library_warnings`, for example
  OpenFC {lib_symbol_issues 2, lib_symbol_mismatch 23} against
  {footprint_link_issues 86, lib_symbol_issues 303, lib_symbol_mismatch 17}.
- The only other field that differs is the DRC's `library_metadata_warnings`
  (its `lib_footprint_*` warnings).
- The whole check (ERC plus DRC) drops from 8.7-29.2 s / 20-50 CPU s /
  2.2-2.5 GB to 3.7-21.5 s / 3.4-27 CPU s / 0.3-0.8 GB.
- The walkthrough test's parity assertions pass. New unit test:
  `erc_library_warnings_are_not_design_issues`.
- `cargo test --release -p pcb-kicad` (345) and `-p pcb-maker --test
  cli_exit_codes --test agent_walkthrough` (7 and 2) pass.

Still on the user's configuration, unmeasured: the import's `sch export
netlist` (its netlist's `libraries` section may name the user's library
URIs), the preview SVG export (`verification_preview.rs`), the fab exports
(`fab_export.rs`), and the other `kicad-cli` calls in lib.rs (lines near
2881, 4407 and 13627 at c9fe4de).

## Committed on 2026-10-15 (import footprints from the board)

Open problem 9's items 1, 2 and 4: personal footprint libraries that are
not in the repository, library drift, board-only parts with nets.

**`import-kicad-netlist --footprints-from <board.kicad_pcb>`**
(`crates/pcb-kicad/src/board_footprints.rs`, `netlist_import.rs`):
- A footprint no library has is taken from that board: the symbol's own
  footprint (by symbol path, else by a reference one footprint has whose
  path is no other symbol's), else any footprint of that name. It is
  brought to library form as KiCad's "Export Footprints to Library" does:
  a back footprint flipped (`flip.rs`), pad and text angles made relative
  (a board keeps them absolute), zones moved into the footprint's frame,
  links, nets, `locked` (also KiCad 7's `fp_text reference locked`) dropped.
  It is written to `<out>/<library>.pretty` with an `fp-lib-table` entry
  (an entry of that name that resolves nowhere, a designer's own folder, is
  replaced), or to `from_board.pretty` when a library of that name exists
  but lacks it (renamed in the copied schematic). Report:
  `footprints_from_board`. A symbol without a footprint takes the board's.
- Where the board has the symbol's footprint, the board's version is used
  over the library's (`board_versions`): the board was made and checked
  with it, the library changed since (KiCad 10's ESP32 modules with the
  antenna clearance in the courtyard, redrawn hotswap sockets). Where the
  board's footprint has another name than the schematic's, the board's is
  used under its name (`substituted`, "(the board's SW0, which has another
  footprint than the schematic's)"); the copied schematic's rename takes
  the most common new name per old id (it names a footprint once per id:
  bumwings' one 1.5u key on a 1u footprint stays a parity finding).
  `drifted_footprints`: per footprint id, the library versions whose body
  (courtyard, else pads) differs from the board's by more than a fifth on a
  side, or whose pad count differs, with both sizes. Without the option
  the import is unchanged.
- Check of the inverse transform: the designer's boards with our written
  libraries in their table, KiCad's `lib_footprint_mismatch`: 29 of 2598
  footprints on 46 boards differ, each a footprint the designer edited on
  the board apart from its other copies (the library is written from the
  first one).

**`add` in constraints.json** (`KiCadAddConstraint`, docs/constraints.md
"Parts the schematic does not have"): `part`, `footprint` (a library's) or
`from_board` (the `--footprints-from` board's `part` or `board_reference`;
a repeated reference is chosen by `x`, `y` absolute), `nets` per pad,
`x`/`y`/`angle`/`origin`, `side`. The import reads the existing
`constraints.json` of the output directory and puts the parts on the board
`board_only` (report `added`); a `from_board` part without `x`/`y` stays
where that board has it, locked. The layout (`expand_additions` in
`place_kicad_board`) turns an `add` with `x`/`y` into a `place` and refuses
(exit 2) an `add` part the board lacks: "import again". Chosen over adding
in the layout because the layout reads the source board in several places
(outline sizing, net classes, the designer comparison); the import writes
the board anew anyway and keeps `constraints.json`.

**The schematic tier** imports with `--footprints-from` the designer's
board and turns the designer's footprints with nets but without a symbol
into `add` entries (`schematic.additions`: from the board, at the
designer's spot, nets renamed to the imported board's by the matched pads,
a repeated reference renamed `REF**_1`), then imports again. Rows have
`from_board`, `board_versions`, `drifted`, `parts.added`.

**Import of all 79 harvested schematics**
(`experiments/schematic-import/import_all.py`, the tier's import step and
library step without the layout; `build/schimport/x10{33,71,72}`):

| | x1033 (before) | x1071 / x1072 (`--footprints-from`, `add`) |
| --- | ---: | ---: |
| imported | 79 | 79 |
| complete (no footprint missing) | 34 | 77 |
| missing footprints (boards) | 1740 (45) | 10 (2) |
| taken from the board (`footprints_from_board`) | - | 2120 |
| board versions used | - | 11155 |
| drifted footprint ids (parts, boards) | - | 67 (453, 31) |
| designer's parts with nets the import lacks (boards) | 2970 (52) | 1253 (16) |
| added by `add` | - | 1253 |

Before, the designer's parts with nets the import lacks are mostly the
missing footprints' parts; after, 1113 of the 1253 are the two panels'
(Batch_1 402, SmartSpin2k 711: their schematic is not the panel's, it
imports no footprint, the panel's parts are all added) and 140 on 14
boards are board-only parts proper (OpenESC 4in1 47 and 4in1-mini 21
copies of parts, Quanta75's two boards 23, 0xcb-static 11, interchange 8,
Sisu 7, bumwings core 6, USB_Keypad and WLED_Matrix 4 holes each,
TelemetryOnboard 3, ...).
The 10 still missing: video's 8 (U25-U27 and others, `kicad_api_lib`, not
on the designer's board either) and bumwings core's 2 (`dao-choc-ble`, not
on its board). `unmatched_pins` 32 to 47: the board's versions lack pads of
no-connect pins (KiCad 10's footprints have them) and, on Quanta75's USB-C
receptacles, pads numbered otherwise (the import says so per part). Drift,
largest: bumwings' hotswap/solder switches (54-58 parts on six boards),
Glasgow's SOT-363 (17), OpenESC's QFNs (pad count), vaio_re's crystals and
SOT-23s, protoesp's horizontal sockets (12.3 x 13.7 against 1.7 x 11.9).

**Schematic tier** x1071 (branch on 8a1fd2c) against `build/sch-x1033.log`,
6 jobs x 4 threads both, under the lock: **5/12 pass both** (x1033 1568 s
wall, x1071 1168 s). bumwings passes (81/81, the board's 12.85 x 11.6 mm
solder footprints instead of the library's 15.9 x 12.2 mm hotswap
sockets); slab-pcb-left fails (74/76, KiCad 2: same body sizes and pads,
an older library drawing; the placement moved 53 of 129 parts); OpenFC
loses its 7 new errors (clearance 3, annular_width 4) with the board's
WSON, TF socket and USB-C, still 80/82; USB_Keypad gets its four GND
holes (37/39 as before, 94 vias against 77); 0xcb-static keeps passing with
its 11 alternative switch footprints added; TelemetryOnboard 3 to 2 KiCad
unconnected with 3 test holes added. Every row in the tier README.
Parity 31 to 40 (OpenFC 5 no-connect pins, bumwings 7 footprint names).

Tests: `board_footprints` (a turned back footprint to library form and
back, drift), `netlist_import`
`footprints_come_from_the_board_where_no_library_has_them_and_the_board_version_wins`
(missing from the board, library written and named in the table, the
board's version and drift, `add` from the board and from a library, the
layout's `expand_additions`, the input errors), `cli_exit_codes` (a
missing `--footprints-from` board exits 2). `cargo test --release -p
pcb-kicad`, `agent_walkthrough` and `cli_exit_codes` pass. Snapshots
x1070 (first cut), x1071 (the measured branch on 8a1fd2c), x1072 (rebased
on 9073633, the 79-board import identical to x1071; no tier on it).

## Committed on 2026-10-15 (hatched pour work)

Question: on main's constants (18 / 26 / 42 ns), do pour analyses on
hatched boards still run far from their modelled work? Route mode on the
v16 placements, tasks v6, one thread, `PCB_ROUTER_POUR_TRACE`, x952 (main
6099e13), all eight boards at once (load 12-24); "model / wall" is the
traced analyses' modelled work over their wall time, the profile column
sums the `profile work:` lines' pour wall and work:

| board | zones | analyses | wall s | model s | model / wall | profile pour wall / work s |
|---|---|---|---|---|---|---|
| Batch_1 | hatched | 442 | 282.5 | 105.4 | 0.37 | 167.3 / 71.0 |
| WLED_Matrix | hatched | 723 | 44.3 | 61.5 | 1.39 | 38.7 / 53.5 |
| USB_Keypad | hatched | 571 | 38.2 | 53.5 | 1.40 | 33.7 / 46.8 |
| Qfwfq | hatched | 309 | 20.9 | 19.2 | 0.92 | 20.5 / 19.0 |
| 1337-v4.0 | hatched | 452 | 14.4 | 10.2 | 0.71 | 14.1 / 10.0 |
| A13 module | hatched | 2250 | 31.2 | 41.1 | 1.32 | 26.1 / 34.8 |
| Castor | solid | 499 | 34.6 | 33.7 | 0.97 | 29.1 / 28.1 |
| Sisu | solid | 1154 | 50.2 | 51.1 | 1.02 | 40.3 / 43.3 |

The controls sit inside 0.8-1.2, the hatched boards outside on both sides
(a repeat with the trace build, x1011, load 13-32: Batch_1 0.34, WLED
1.60, Keypad 1.83, A13 1.76, 1337 0.81, Castor 0.99, Sisu 1.05). Batch_1
is still about 3 times under on main (the v16 triage's 755 / 200 s was a
four-thread layout on x862's constants). What the items-only model
misses, from trace counters per analysis (thread-local: route mode runs
two ladder rungs side by side even at one search thread, so process-wide
counters mixed them, as x1010's did):

- The probe's queries (a point tested against its index, `blocker` and
  `slack`). A query costs about a microsecond whatever its items (the 9-25
  bucket lookups, the `edges` vector, `inside`'s nearest-node and polygon
  test); a hatched probe asks many more of them, with few items each in
  open fill: a hatch hole judged (`hole_kept`, memoised per probe; 1.8-2.0
  k on WLED and Keypad) samples 64 points with `fill_at` and `near_edge`,
  up to some 2000 queries, and the ring test (`in_ring`) up to 33 per pad
  of the net (WLED 24 M queries, 400 k ring tests). Per
  item the probe's wall time was 21-34 ns on WLED and Keypad (many items
  per query) and 51-81 ns on 1337, Qfwfq and Castor; with a query term
  it is one model.
- Polygon vertices. Batch_1's queries cost 5.5 microseconds and its items
  220 ns: its board cutouts are `Hole` obstacles of up to 134 vertices
  (x1012 dump), and every shape test near them walks the polygon twice;
  15.9 G vertices in 442 analyses. Not the hatch itself, but the hatch
  multiplies the queries near them. The ring test's own pads are 0.8 G of
  those (x1014), `inside`'s polygon fallback 0.1 G: not it.
- So the hole grid and the rings cost through their queries, not as a
  term of their own; the per-zone seam counting (`in_zone_part` on
  4-point outlines) is small. The
  lattice part (fill mask, labelling, stamps) was over-counted instead:
  A13's analyses spend 17.6 s outside `kicad_spokes` for 41.1 s of node
  and stamp work (486 k stamps at 26 ns), Sisu 38.2 for 51.1, which the
  items-only fit balanced against `kicad_spokes`' uncounted rest.

Change (`fill::FillProbe` counts `queries` and the `vertices` its shape
tests go through, `Router::probe_nanos`): `PROBE_QUERY_NANOS` 850,
`PROBE_VERTEX_NANOS` 9, `PROBE_ITEM_NANOS` 42 to 9, `POUR_NODE_NANOS` 18
to 23, `POUR_STAMP_NANOS` 26 to 6. Fitted (least squares of the log
ratio, every board one weight) on the eight boards and the four other
boards of the x816 calibration (reCamera, k30-SBC, USB_Cable_Tester,
spell_tome; x1013, route mode at budget scale 0.3, load 10-17): per board
model / wall Batch_1 1.03, WLED 0.97, Keypad 0.97, Qfwfq 1.01, 1337 0.91,
A13 1.01, Castor 1.02, Sisu 0.89, reCamera 1.12, k30 1.05,
USB_Cable_Tester 1.00, spell_tome 1.41 (1.1 s of analyses; it was 1.11
with the old constants). The pour trace now prints `probe queries`,
`probe vertices`, `probe work` and whether the net has a hatched zone.
Test: `a_fill_probe_counts_its_queries_and_polygon_vertices`;
`pour_analyses_count_as_budget_work` takes the probe's whole work.

Judge, route mode, the same eight boards one run per side, x952 against
x1016 (6099e13 with this commit), one thread; the after side at load
22-24:

| board | result (both sides) | model / wall before, after | clean-ups: work (pour) of budget in wall s, before; after |
|---|---|---|---|
| Batch_1 | cut (budget guard), 499 open, 345 vias, 5180.1 mm | 0.37, 0.87 | 57.2 (19.5) of 293 in 71.5; 73.3 (35.6) of 293 in 58.7 |
| WLED_Matrix | 6 open, 143 vias, 2139.8 mm | 1.39, 0.86 | 55.1 (27.3) of 240 in 37.5; 47.9 (20.1) of 240 in 46.0 |
| USB_Keypad | 2 open, 79 vias, 1555.7 mm | 1.40, 0.86 | 93.7 (30.6) of 360 in 61.9; 83.1 (19.9) of 360 in 66.7 |
| Qfwfq | 0 open, 62 vias, 3787.2 mm | 0.92, 0.81 | 83.9 (10.8) of 240 in 72.3; 85.1 (11.8) of 240 in 87.7 |
| 1337-v4.0 | 0 open, 66 vias, 1578.7 mm | 0.71, 0.83 | 41.9 (5.4) of 240 in 35.7; 43.9 (7.5) of 240 in 43.9 |
| A13 module | 14 open, 411 vias, 6015.1 mm | 1.32, 0.96 | 99.0 (27.9) of 196 in 68.9; 86.1 (15.3) of 196 in 50.3 |
| Castor | 0 open, 262 vias, 5593.9 mm | 0.97, 0.86 | 156.8 (21.0) of 209 in 132.5; 157.8 (21.9) of 209 in 146.1 |
| Sisu | 6 open, 388 vias, 5897.0 mm | 1.02, 0.90 | 170.1 (36.1) of 189 in 131.3; 165.9 (27.8) of 189 in 107.4 |

Every board's result is identical; the analyses' counts too but Sisu's
(1154 to 1156, its work 1455.3 to 1459.5 s): the clean-ups here end on
their passes, not on their budgets, so the counted work moves (Batch_1's
pour share 19.5 to 35.6 s, the hatched 1.3-1.4 boards' down by a quarter to a half)
without moving a decision. The after side's ratios sit 0.81-0.96 at the
higher load (the spread, 0.15, is what matters: before 0.37-1.40).
Batch_1's cut is the same on both sides and not the pour work: "budget
guard: 0 s of wall clock for a budget of 0 s of work", a phase with a
budget under half a second whose first analysis runs past four times it
before the work notices (one Batch_1 analysis is about a second of wall).
Its route-mode result (107/204 routed, 499 open) is that board's v16
placement, not this change.

Short tier (tasks v6, `--jobs 2 --threads 4`, under the lock, load 15-17)
x1015 (8c20715 with this commit) against build/short-x1041.log (main's
code): 12 of 12 rows identical (vias, length, work, KiCad; unconnected 35
to 35, errors 3 to 3, passes 6 to 6, work 2671 s on both). No clean-up
budget on the tier's hatched boards (1337-v4.0, USB_Keypad, WLED_Matrix)
ends by work. Against build/short-x941.log the rows differ by main's own
changes since (neck repair, placement work), not this one.
`cargo test --release -p pcb-router -p pcb-kicad` passes. Logs:
build/hatchwork-* (x952), build/hwtrace-*, build/tl-*, build/poly-*,
build/vx-*, build/val-*, build/ring-batch1 (traces), build/after-*
(x1016), build/short-x1015.log.

## Committed on 2026-10-15 (wall guard stop, fewer layout checks)

Three items of the v16 triage ("Ranked": 2, 3, 4) and the work of an
unkept ladder (smaller hole 4). Binaries: x972 (the change on 9ce95b1, for
the layout pairs against x934), x976 (rebased on bd0babc: neck repair,
reserve floor; short tier), x977 (rebased on 8a1fd2c, the commit's code).

**Stop after the wall guard** (`WallStop`, `LayoutStep` in board_layout.rs).
Once the layout's wall-clock deadline passes, only the steps that make the
result run: labels, KiCad's check of the board kept, the report. Skipped,
each noted as a `layout` cut and logged once (`layout: past the deadline,
... skipped`): the placement race, move trials (checked before every
trial, not only every move: v16's poly_kybd_split72_left ran three trials
of 60-200 s past the guard and was killed), the resume, the polish, the
final ladder, the attempt after the narrow-signal attempt and the neck
repair (up to six KiCad checks; also skipped in the ladder past its
deadline, and its checks are counted apart in the log). In the router, a
negotiation that resumes an earlier one (resume, trial, via reduction)
starts no iteration past the deadline, and the pour stitching leaves
stranded pads instead of their hard reroute; a fresh router still
negotiates once. Without a cut nothing changes (every skip needs the wall
clock past the deadline). Checked on poly_kybd_split72_left, tasks v6,
4 threads pinned to two cores (`taskset`, beside other agents, load 14-27):
full budget x970 ended at 1719 s (harness 1800 s) with a result, 29
unconnected, 981 vias, cut deadline+layout: the guard fired in the
narrow-signal attempt's forced repair, the attempt finished and was
checked (it could win and won), the continuation was skipped, `layout:
done` at 1688 s. At `--budget-scale 0.3` (timeout 750 s, deadline 495 s)
x970 and x934 both ended at 520-525 s (cut in the first route, 368 and
483 open after it, so no moves or ladder to skip). Unit tests: a fake step
list (`past_the_wall_guard_only_the_finishing_steps_run`,
`once_fired_the_guard_stays_fired`).

**Fewer KiCad checks in the ladder** (`CheckGate` in board_router.rs;
docs/router.md "The attempt ladder"). Attempts route without KiCad's check
and are checked only when they could still win. The ladder's open count is
the router's (unconnected terminals and internal violations) plus KiCad's
unconnected items, so the router's count is a lower bound with no slack:
an attempt whose router count is above the best checked open count (or
equal, with no starved thermal on the best and no fewer vias and copper)
cannot win. The ladder's first board and any board the router finishes
are always checked; a seed retry against an unchecked rung prefers the
checked board (exact otherwise). Route mode uses this rule alone (no
decision of the ladder changes). The layout's final ladder (and the
narrow-signal attempt and its continuation) also checks only attempts
among the two best by the router's measures (open count, vias, copper),
the layout's own board counted first (`KICAD_CHECK_TOP`); a first version
ranked by the open count alone and lost MokyaLora's 333-via narrow step
(339 kept), fixed by the vias tie-break. Past the deadline an attempt is
checked only if it could beat both the ladder's best and the layout's
board (`kicad_kept`), and an unchecked ladder board is never kept. Log:
`board check: KiCad skipped, ...` per attempt, `layout: done (...); KiCad
checks N (from the DRC cache, of the neck repair), M skipped`. Unit tests
`check_gate_tests` (5).

**Unkept ladder work counts**: the layout's `routed.work_seconds` is the
ladder's work (and the continuation's) whether its board is kept or not,
as the narrow path did since 974fa2b (koeg in v16 reported 410 s with
1100 s more run). Rows' `work_seconds` rise where a ladder ran and lost;
results do not change.

**run.py threads**: the default with several jobs is the physical cores
(from /sys, else lscpu) over the jobs, at most 4, at least 1 (was the
logical CPUs: 5 jobs got 4 threads on 16 cores, now 3; 6 jobs got 4, now
2). An explicit `--threads` is kept; jobs x threads above the physical
cores is warned about. `summary.json` records `jobs`, `threads`,
`physical_cores`. A layout is deterministic per thread count: compare at
the same `--threads` (give `--threads 4` against older short and quick
tier logs).

Layout pairs, tasks v6, 4 threads, one run per side, PCB_MAKER_NO_CACHE=1,
x934 against x972 side by side, load 9-39 (rows in build/guard-ab-x934.log
and build/guard-ab-x972.log; the poly_kybd runs in build/guard-poly-*.log). KiCad checks are the DRC calls
of the layout (seconds of kicad-cli in brackets):

| board | unconnected | errors | vias | work s | wall s | cut | KiCad checks |
|---|---|---|---|---|---|---|---|
| ghoul | 6 / 6 | 0 / 0 | 385 / 385 | 1208 / 1208 | 1088 / 1079 | no / no | 7 (188 s) / 4 (88 s) |
| 0xCB panel | 109 / 109 | 0 / 0 | 231 / 231 | 362 / 362 | 1298 / 1052 | no / no | 10 (331 s) / 4 (163 s) |
| Batch_1 | 491 / 491 | 0 / 0 | 384 / 384 | 1156 / 1156 | 1262 / 1377 | no / no | 5 (203 s) / 2 (108 s) |
| ULK | 4 / 4 | 0 / 0 | 130 / 130 | 98 / 98 | 1679 / 1572 | deadline / no | 13 (370 s) / 7 (205 s) |
| woagboard | 5 / 5 | 1 / 1 | 185 / 185 | 1131 / 1540 | 1665 / 1641 | deadline / no | 12 (127 s) / 8 (70 s) |
| MokyaLora (pass) | 0 / 0 | 0 / 0 | 333 / 333 | 1365 / 1365 | 944 / 812 | no / no | 7 (61 s) / 3 (22 s) |
| zpn_devboard (pass) | 0 / 0 | 0 / 0 | 417 / 417 | 522 / 522 | 595 / 466 | no / no | 1 / 1 |
| Castor mainboard (pass) | 0 / 0 | 0 / 0 | 167 / 167 | 924 / 924 | 829 / 721 | no / no | 2 / 2 |

All eight boards identical (unconnected 615, 1 new error, 2232 vias,
47510 mm both); KiCad checks 57 to 31, kicad-cli 1297 to 676 s; CPU 14495
to 12643 s. Cut 2 to 0: on x934 the wall clock cut woagboard's and ULK's
last rung (both boards unchanged; woagboard's counted work 1131 against
1540 s is where the cut stopped that rung, ULK's 98 s leaves out its
unkept ladder, the accounting fixed above came later). Wall clock follows
the load (Batch_1 ran beside load 22-26 on x972, 9-23 on x934); the
checks saved are 100-170 s a board on the big ones.

Short tier (flock, tasks v6, `--threads 4 --jobs 1`, DRC cache warm), x976
against build/short-x1020.log (x1020 = x950's rows; x1022 changes none):
all 12 rows identical (unconnected 35, new errors 3, 1129 vias, 22253 mm,
passes 6, cut 0). Work 2671 to 3236 s: framework_mobo 220 to 292 and OpenFC
140 to 634, their unkept ladders now counted; every other row's work the
same. KiCad checks 62 to 41 (rung checks 53 to 32; the neck repair's 7 on
USB_Keypad and 2 on WLED_Matrix both sides): USB_Keypad 20 to 14,
reCamera 14 to 4, WLED_Matrix 11 to 9, OpenFC 6 to 3, the rest unchanged.
Wall 1702 to 1946 s summed over the rows (the cache made the checks
0.5 s each, so the saving is nil there; jobs 1 beside other agents' tiers
against x1020's run). build/short-x976.log.

Not done: the uncached KiCad check's time is still not work (the gate cuts
the number of checks, not their charge; done since: "KiCad check work"); the ladder's first board is always
checked (it is the ladder's measure), even where it cannot beat the
layout's board.

## Committed on 2026-10-15 (schematic tier)

The agent's real use case as a benchmark (open problem 8, item 7): the
harvested boards' schematics, and a tier that starts from them.

**Harvest.** `benchmarks/github/harvest_schematics.py` fetches, at each
board's origin commit, the root sheet and every sheet it names, the
project file and `.kicad_dru`, the library tables, the symbol libraries the
tables keep in the repository and the footprints the symbols name from the
repository's footprint libraries (Git submodules followed: 19 boards keep
their libraries in one). Libraries a table names by a designer's absolute
path (`/home/seb/dev/...`), or that lie outside the project folder, are
found in the repository by path suffix, folder or file name and recorded as
`library_fixes` (22 boards). Files go to
`benchmarks/real/external/github-sch/<board>/repo/` (gitignored), the
per-board record to `schematic.json` there and to the committed
`benchmarks/github/schematics.json`. Licences: GitHub's SPDX id where it
has one; `NOASSERTION` and none are decided by reading the licence files
at the root and along the board's folder (CERN OHL, AGPL and Hippocratic
texts GitHub does not name were allowed; no licence file at all means all
rights reserved and the board is skipped). **79 of the 109 boards have a
usable schematic** (241 sheets, 1171 files, 95 MB). Skipped: 17 without a
licence file (ISS-PCB's six, seven OnBoard boards, MegaDrive, Neotron-Pico,
SUMEC, ATAT1800, vd-rd's module), 10 without a schematic of the board's
name (PolyKybd's five molecules, the OpenESC, OpenRX and two 0xCB panels,
krishveercard, whose sheet is misspelt), 2 with a sheet missing at the
commit (the Hub, poly_corne_split42_right), 1 KiCad 5 schematic (Qfwfq).
50 of the 79 have every named footprint in KiCad's libraries or the
repository.

**Imports of all 79** (x1002, import and constraints only): 76 imported,
3 failed (osprey, hackrf, rosco_m68k: KiCad 5 arcs, below); 33 complete
after the library step (2 of them only through it), 43 with footprints
missing: personal libraries that are not in the repository (Castor's
`winterbloom` 160, DSP-ADAU1452 371, jetson's `antmicro-footprints` 121,
reCamera's `Capacitor`/`Resistor` 92, OpenMuscle's `FlexSensorFootprint`
60, ligra's `otter` 33, ...), hackrf's KiCad 5 `.mod` library (395),
footprints a KiCad library no longer has (OpenESC's `C_0402_WIDE`,
Blue_Line's `CONN3_S3B-PH_JST`), symbols without a footprint (framework
mobo's RN1). Two panels (Batch_1, SmartSpin2k) import empty: their
schematic is not the panel's.

**The tier** (`run.py --tier schematic`, `benchmarks/agent-tasks/schematic.py`,
`schematic.json`; README.md, "The schematic tier"): per board, import the
copied project with the designer's layer count, add the found libraries and
import again when footprints are missing, move tasks v7's constraints onto
the new footprints (matched by symbol path, else by reference; `fixed`,
netless and locked parts as `place` at the designer's spot; every part on
the designer's footprint layer with `back`/`front`), draw the designer's
Edge.Cuts, copy the netless board-only footprints (fixed, `board_only`),
`layout-kicad-board` at a third of the budget, `drc-kicad-board`. Pass:
the layout rule and no footprint missing; parity counted. Rows have
`stopped` (import, constraints, placement, routing, drc) and `why`.

First run on x1002 (4 jobs x 2 threads, load 9-21, `build/sch-x1002.log`):
4/12 pass in 1034 s wall; stopped at routing 6, drc 2. Passed:
spell_tome, 0xcb-static (after the library step), slab-pcb-left,
homebrew_486, each as in the layout tier. The other rows are in the README.

**Fixed in the branch** (x1030 = this branch's binary):
- *KiCad 5 library footprints*: `(module ...)` files were imported under
  that head; pcb-maker reads only `footprint`, so the parts stayed at the
  import's point, unplaced and unrouted, without a word (coco2's 16
  footprints: 87 unconnected and 199 shorts; OpenFC's SD socket and a
  sensor: 17 unconnected). The import writes `footprint`. Arcs given by
  centre and angle (`fp_arc (start c) (end s) (angle a)`) became a board
  KiCad 10 cannot load (kicad-cli exit 3, no message; the import's pad check
  failed with exit 4 on osprey, hackrf and rosco_m68k); they are converted
  to start, mid and end. Tests `kicad_5_arcs_become_three_points`,
  `kicad_5_library_footprints_become_footprints_of_today`.
- *Libraries outside the project were copied outside the output*: a
  `${KIPRJMOD}/../lib` entry was copied to `<out>/../lib` (the survey
  found four library folders written beside its output directories). Only
  libraries inside the project are copied now.
- *`back` reported missed on a board that kept it*: urchin's key sockets
  are front footprints with their pads on B.Cu; `back` flips them to B.Cu
  as asked, but the kept check compared the part's side by its pads
  (front) with "back": all 34 were reported missed (exit 1). The side to
  keep is now the one the flipped footprint's pads give, as lowered.
  `describe-kicad-board` lists `layer` (the footprint's own) beside `side`
  (where its pads are), so an agent sees which one `back` sets. Test in
  `constraints_kept_by_construction_are_reported`.

Rerun on x1030 of the boards the fixes touch (coco2, OpenFC, 0xcb-static,
spliced into the x1002 log, `build/sch-x1030-three.log`, after a
harvest refresh): **5/12 pass**. coco2 173/173 routed, KiCad 0 unconnected,
no new error, 614 vias, passed (x1002: 87 unconnected, 199 shorts; the
layout tier's v16 row 173/173 pass); OpenFC 80/82, KiCad 2 unconnected,
clearance 3 and annular_width 4 (x1002: 17 unconnected; layout tier 81/82,
1); 0xcb-static passed again, now with its library found in the project
folder on the first import (the refreshed harvest fetched the copy nearest
the project), 43 footprints substituted by name, which showed one more
gap: parity 44 (`footprint_symbol_mismatch` 43).
- *A substituted footprint kept its old name in KiCad 6 schematics*: the
  import rewrote the symbol's `Footprint` property in the copied
  schematic, but a KiCad 6 file repeats it per instance
  (`symbol_instances`, `(footprint "MX_Only:...")`), and that one wins.
  Both are rewritten now (x1031): the imported 0xcb-static's parity check
  (kicad-cli `--schematic-parity`) goes from 44 findings to the 1
  no-connect pin.

Unit tests (pcb-kicad netlist_import, placement_constraints, describe) and
`tests/agent_walkthrough.rs` pass. Snapshots `build/bin/pcb-maker-x1030`
(the rerun) and `x1031` (with the schematic rewrite); `x1032` is the
branch rebased on 8c20715 (main's x1042).
No layout tier was run on x1030: the import fixes change only boards with
KiCad 5 footprints, which no layout task has (their boards come from the
designer, written by KiCad); the side check changes the report (and so the
exit code and the pass rule) only where `back`/`front` name a footprint
whose pads are on the other side, which no task does.

## Committed on 2026-10-15 (outline polygons)

Gap 5 of the walkthrough (open problem 8): `outline` in constraints.json
took only a rectangle. Snapshots: x1040 = main at 851a473 (the base),
x1041 = this.

**What an agent can write** (docs/constraints.md, "Outlines that are not
rectangles"; agents.md's cookbook):
- `"polygon": [[x, y], ...]` (mm, closed implicitly, its points' box
  moved to `x`, `y`), checked: three corners, an area, no side crossing
  another (input errors, exit 2);
- `"corner_radius"` on the rectangle, the polygon, a piece or a cutout:
  every corner (inner ones too) an arc, the radius cut where a side is
  too short (a 30 x 4 mm outline with radius 5 is a slot with round ends);
- `"cutouts"`: boxes or polygons relative to the outline's `x`, `y`
  (`"origin": "absolute"`), inside their outline and apart from each
  other;
- `"pieces": [outline, ...]`, each with its own offset, radius and
  cutouts; they may not overlap or touch. No tabs are drawn.
A rectangle without radius or cutouts takes the old path (one gr_rect);
the automatic size and `shrink` stay rectangles (`area_factor`, `aspect`,
`shrink` with a polygon or pieces are errors; cutouts need a size).

**Writing** (`apply_shaped_outline`, placement_constraints.rs): every
contour as `gr_line` and `gr_arc` (start, mid, end; nanometre-rounded so
the pieces chain), stroke 0.05 mm, replacing the board's Edge.Cuts
graphics as before. `outline::board_loops` reads them back (the router's
and the placer's boundary, unchanged), and now also keeps the pieces'
own loops (`piece_loops`) and which cutouts the board's own graphics draw
(`board_cutouts`, as opposed to a footprint's slot).

**The placer.**
- *Edges on a polygon* (`Constraints::sides`, set by the lowering when
  the constraint's outline or one of its pieces is a polygon): each
  segment of the pieces' loops faces the side its outward normal points
  along most (`outline_sides`; 45 degrees counts as top or bottom). An
  edge constraint holds the part at the nearest segment facing its edge,
  by box-to-segment distance (`side_lines`): the line is the segment's
  innermost coordinate over the body's stretch (a slanted side: where the
  body's corner meets it), and a body not alongside the segment misses by
  the gap (`hard_violation` adds it). `edge_frame` takes the body's half
  extent for this; every caller had it. `any` measures the same way
  (`side_distance` in `choose_any_edges`). Rectangles, rounded ones, ones
  with cutouts and pieces that are all rectangles keep the box (or the
  piece boxes): nothing changes for them. On urchin's L (below)
  SW_POWER1 "top" goes to the step at y = 30; against the box it would
  have had no place in the right half.
- *Cutouts keep bodies off* (`Constraints::solid_cutouts`): cutouts the
  board's own graphics draw when the constraints give an outline (the
  outline constraint replaced all others) are tested against bodies by
  their polygon in `clear_of` (`body_meets_solid_cutout`; touching is
  allowed) and penalised by their box in the anneal; a footprint's slot
  still keeps only copper off (a connector's pegs).
- *Room*: the usable area subtracts the solid cutouts' area (the polygon's
  own area was already used); `outline_mm` and the suggestions stay boxes.
- With an outline from the constraints, a part whose centre lies off a
  polygon (in an L's notch) counts as off the board (placed, not kept).
- Pieces feed `keep_on_pieces` and the per-piece edge frames as a board
  drawn with several pieces did.

**Test boards** (urchin, the walkthrough's fixture and constraints, 30 s
of work, four threads, `PCB_ROUTER_NO_WALL_GUARD=1`): spell_tome's
schematic is not in the repo (its licence is NOASSERTION, and its
artwork footprints are 4 MB), so the test uses urchin (MIT).
- L-shaped polygon, 230 x 125 mm box less a 115 x 30 mm notch (the right
  half 30 mm lower), each half's region in its part: exit 0 in 15 s
  (18 s of work), 68/68, KiCad 0 unconnected, 0 new errors, every
  constraint kept; SW_POWER1 (edge top) at y = 30.02, the step.
- 230 x 95 mm with 5 mm round corners, a 10 x 55 mm slot between the
  halves and a triangular window: exit 0 in 23 s (30 s of work), 68/68,
  0 unconnected, 0 new errors, no body in the slot; four gr_arc.
- drc-kicad-board on both: exit 0, no `invalid_outline`, no
  `copper_edge_clearance` (only the fixture's library findings, and two
  silk_edge_clearance warnings on the rounded board).
- `"edge": "any"` on the L: chosen by the nearest facing side, kept.

**Tests**: `outline_shapes_parse_and_mistakes_are_errors`,
`outline_shapes_are_written_as_edge_cuts_lines_and_arcs`
(placement_constraints), `a_shaped_outline_gives_the_placer_its_sides_pieces_and_solid_cutouts`
(board_placer), `a_part_held_at_the_top_of_an_l_keeps_to_its_step`,
`a_cutout_the_user_cut_keeps_bodies_off` (pcb-placer tests), and
`the_keyboard_lays_out_on_an_l_and_on_a_board_with_round_corners_and_cutouts`
in tests/agent_walkthrough.rs (both layouts above, about 35 s).

**Identity** (rectangles: the tasks give no `outline`, so no new path
runs). place-kicad-board with the short tier's prepared sources and
layout.json (build/short-x1000, tasks v6, 120 s of placement work, four
threads), x1040 against x1041: byte-identical boards on all 12 short-tier
boards. Short tier on x1041 (tasks v6, 2 jobs x 4 threads, under the
lock, `build/short-x1041.log`) against build/short-x1000.log: compare.py
same 12, unconnected 35 to 35, passes 6 to 6, vias 1129, length 22253 mm,
work 2671 s, first-route open terminals 61, every row field identical
but WLED_Matrix's `native` (connection_width 9 to 0: new errors 12 to 3
in all); every layout's placed board byte-identical, every result board
too except USB_Keypad's and WLED_Matrix's, which carry the neck repair's
added tracks (2 and 12, their layout logs' "connection width" lines).
That is main's neck repair (70279ba, after x1000's base 9ce95b1), not
this change: x1000 predates it, as x941 to x950 measured (12 to 3).

## Committed on 2026-10-15 (minimum connection width)

KiCad's board setup has `min_connection` (minimum connection width) and
the project a severity for connection_width findings; the router chose
widths from the classes, the narrow-signal step's menu width and the neck
width without regard to it. Now `project_rules.rs` reads both
(`min_connection_mm`, `connection_width_severity` in the router config;
KiCad's default severity is a warning, board_design_settings.cpp) and,
where it applies, floors every track width: the class widths, the
narrow-signal width (the menu's smallest width raised to the floor), the
neck width; `route_kicad_board` and the lowering raise them again
(`apply_width_floor`, `width_floor`), so a configuration file's rules and
requested net classes keep it; stubs, escapes, manual and inner spokes
take their net's class or neck width; the neck repair's tracks already
were the minimum rounded up to a micron. main.rs's file merge copies the
floor from the project. `board-router.json` has a new `rules` object:
`width_floor_mm`, `connection_width_severity`, `narrowest_track_mm`,
`neck_width_mm` (WLED: 0.127 / error / 0.127 / 0.127).

The choice: the floor applies at error severity only, by default
(`PCB_MIN_CONNECTION_FLOOR=warning` adds warnings, `off` none). Measured
with warnings floored too (x1020), it cost routing on the only boards
where it matters (below). Census of the 109 v6 tasks
(`experiments/min-connection/census.py`, mirroring project_rules as of
x952; `emitted.py` counts the routed tracks below the minimum in a run):
18 have a `min_connection`, all with an explicit severity (3 error, 15
warning); a router width below the minimum only on two, both warnings:

| board | min_connection | severity | narrowest class | narrow step | neck | below the minimum |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| OpenESC-20x20 4in1-mini | 0.16 | warning | 0.09 | - | 0.1 | Default 0.09 (every track); neck 0.1 |
| OpenESC-30x30 4in1 | 0.16 | warning | 0.09 | - | 0.1 | Default 0.09 (every track); neck 0.1 |
| OpenESC-30x30 4in1-panel | 0.16 | warning | 0.16 | - | 0.16 | - (no v16 result) |
| USB_Keypad, WLED_Matrix | 0.127 | error | 0.127 | - | 0.127 | - |
| USB_Cable_Tester | 0.1 | error | 0.1 | - | 0.1 | - |
| MokyaLora | 0.1 | warning | 0.1199 | 0.1199 | 0.1 | - |
| Sisu_SSE-9 | 0.1 | warning | 0.1 | 0.1 | 0.1 | - |
| reCamera | 0.1016 | warning | 0.1016 | - | 0.1016 | - |
| OpenFC, OpenRX panel | 0.09 | warning | 0.09 | - | 0.1 | - |
| 0xCB-1337 pcb, pcb-panel | 0.09 | warning | 0.1 | 0.2 | 0.1 | - |
| BAGEL-MK1, MK1.1-Avocado | 0.1 | warning | 0.127 | - | 0.1 | - |
| OpenAirScope | 0.1 | warning | 0.2 | - | 0.14 | - |
| MegaDrive | 0.1 | warning | 0.25 | - | 0.1 | - |
| SUMEC_MK_IV | 0.01 | warning | 0.25 | 0.25 | 0.1 | - |

On the v16 layouts only the two OpenESC boards have tracks below the
minimum (all 2530 and 2483 of them at 0.09; the designer's are 0.16 and
up, mostly 0.2). Route mode on the v16 placements, tasks v6, one run per
side, 2 jobs x 2 threads (build/minconn-route-x952, -x1020, -x1021;
connection_width counted apart, KiCad's report caps it at 199, necks.py
counts all; no other errors on any side):

| board | side | unconnected | connection_width | vias | length | work |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| 4in1-mini | x952 (no floor) | 52 | 199 warnings (1007 by necks.py) | 157 | 1770 | 835 |
| 4in1-mini | x1020 (floor 0.16) | 89 | 0 | 126 | 1508 | 1172 |
| 4in1 | x952 (no floor) | 104 | 199 warnings (709) | 123 | 2073 | 903 |
| 4in1 | x1020 (floor 0.16) | 121 | 0 | 124 | 2003 | 1111 |

0.16 tracks at the class's 0.09 clearance need a lattice pitch of 0.25
against 0.18: 37 and 17 more open items for the warnings gone, so the
floor is not shipped there. x1021 (the commit, rebased on 851a473, error
severity only) gives x952's rows exactly on both. The error-severity
boards (WLED, Keypad, Cable_Tester) already route at or above their
minimum, so the floor changes nothing in their routing today; it guards
other class or narrow-step widths (a requested net class, the narrow step,
a neck) from going below.

Short tier (flock, tasks v6) x1020 (on 6099e13) against
build/short-x941.log: unconnected 35 to 35, new errors 12 to 3, vias 1129
to 1129, length 22253 mm both, passes 6 to 6, work 2671 s both, cut 0.
The changed rows are x950's neck repair, not this: WLED_Matrix
connection_width 13 to 1 (+11 to +2 new errors), USB_Keypad 4 to 2;
against build/short-x950.log all 12 rows are identical (routed, vias,
length, KiCad's counts, work, first-route open terminals). No short-tier
board has a width below its minimum (reCamera's and OpenFC's classes are
at it), so the warning choice does not move them either. Logs
build/short-x1020.log, build/minconn-route-x{952,1020,1021}.log.

Unit tests: `the_connection_width_severity_decides_the_floor`,
`the_minimum_connection_floors_the_project_widths` (project_rules),
`the_minimum_connection_floors_classes_the_narrow_width_and_the_neck`,
`a_routed_board_has_no_track_narrower_than_the_minimum_connection`
(board_router; lowers and routes a board with a forced neck-down).

Open: OpenESC's warnings (700-1000 necks of our own 0.09 tracks) remain;
a middle way would be the floor where it costs nothing (a last pass
widening tracks to 0.16 where the clearances allow, as the neck repair
does), not built.

## Committed on 2026-10-15 (panel fixes)

The two panel defects of the v16 triage (part 4). x1000 = this on
9ce95b1; x1001 = the same after the rebase on 6099e13 (necks only).

**1. Entries for a repeated reference name their footprint by uuid.**
The task generator writes an edge and rotation entry only for the copies
the designer put at an edge, in board order; x792's in-turn rule bound the
n-th entry to the n-th footprint of that reference, so the entries landed
on other copies. Now an `edge`, `rotation` or `place` entry may carry
`uuid` (the footprint's `uuid`, `tstamp` in KiCad 5-7 files;
`describe-kicad-board` lists it): it binds that footprint, a uuid no
footprint of the reference has is an error, and entries without one keep
the in-turn rule (hand-written constraints; docs/constraints.md).
`from_pcbench.py` writes `uuid` for every entry of a repeated reference.
**Tasks v7** (`build/github-tasks-v7`, the existing generator with
`--corpus` on the absolute-path copy of boards.json, x1000's
describe-kicad-board): tasks.json byte-identical to v6, 103 constraint
files identical, six differ only by uuids (the five molecules and
OpenESC's 4in1-panel, one entry). Binaries before x1000 reject v7's six
files (unknown field `uuid`); the other 103 are v6's.
Judge, every part fixed at the designer's spots (`move_all` off, the
task's constraints, place-kicad-board): missed constraints x990 on v6 /
x1000 on v7: 4x4 29 / 0 (17 of them on J1, J3, J4, the triage's count;
the rest on J5, J6), 4x5 31 / 0, 5x2_shifted 14 / 0, 5x4_wave 10 / 0,
7x5_wave_left 19 / 0. With J5, J6 and J9 free (the rest fixed): 5x4_wave
and 7x5_wave_left complete; 4x4 (J5, J6), 4x5 (J5, J6) and 5x2 (J5)
still find no legal place for some back connectors. That is geometry, not
binding: at the designer's poses (`PCB_PLACER_DESIGNER`) neighbouring
tiles' J5 and J6 lie 0.03 mm apart on the back (centres 2.03 mm, pad
bodies 2 mm wide) and the edge tiles' J5 0.016 mm from the board edge
(the copper-to-edge floor is 0.02 mm).

**2. keep_copies regions follow the copies' layout, and copies start in
their own region.** SmartSpin2k's 57 unplaced parts were the rule's, not
only the start's: its fixed parts sit at the top of each copy (2.3 mm
from the board's top edge, 40.5 mm from its bottom), so the cut halfway
between two copies' fixed parts ran through a copy's own parts: 162 of
the designer's 630 movable parts lay outside their copy's region
(the 3 x 3 copies got rows 50.7, 65.3 and 88.9 mm high). The cut now
moves from halfway by half the difference of the board's margins beyond
all copies' fixed parts, clamped between the two copies (each copy's
region lies around its fixed parts as the board lies around all of
them): 0 of 630 outside, rows of 67.6, 65.3 and 72 mm. The 0xCB pcb-panel's
regions do not change (0.1 mm display), the 0xCB panel's cut moves from
82.7 to 82.0 (the board's middle). In addition each confined part starts
global placement at its region's centre (`Constraints::starts`, read by
`global_place`; parts without one start as before). place-kicad-board on
v16's prepared SmartSpin2k (4 threads): x990 57 parts without a legal place; the shifted
regions alone complete (final wirelength 14217 mm); shifted regions and
starts complete (12219 mm). Only boards where keep_copies fires change:
by a replica of its test over the 109 tasks of v7, SmartSpin2k and the
two 0xCB panels.

**Layouts, one run each** (four threads, jobs of two, load 12-30; tasks
v6 for x990, v7 for x1000; placed = movable parts with a legal place):

| board | before | x1000 |
| --- | --- | --- |
| SmartSpin2k_Panelized | x990: 57 parts without a legal place (exit 1, 185 s) | 630/630 placed, 568/585 routed, KiCad 19 unconnected, 0 new errors (the designer's 199 invalid_outline subtracted), 1036 vias, 1218 s of work, not cut, 924 s |
| poly_kb_molecule_4x4 | v16: 4 unplaced, 8 illegal (J5, J6, SW1) | 157/160 (J5, J6 twice), no constraint missed |
| poly_kb_molecule_4x5 | v16: 6 unplaced, 17 illegal | 190/196 (J3, J5 three times, J6 twice, SW1) |
| poly_kb_molecule_5x2_shifted | v16: 2 unplaced (J5, J6) | 129/130 (J5) |
| poly_kb_molecule_5x4_wave | v16: placed, 2/28 routed, 499 unconnected, 5 starved | placed, 3/28 routed, 499 unconnected, 6 starved_thermal, 501 vias, 937 s |
| poly_kb_molecule_7x5_wave_left | v16: 1 unplaced (J9) | 234/235 (J9) |
| OpenESC 4in1-panel (information) | v16: 207 parts (load 54) | 199 parts without a legal place: crowding (one net group, no copies) |

The molecules now place the designer's fixed parts' edges right, but
from the stack the back connectors still need the designer's 0.03 mm
gaps (above); the next step there is pad-level spacing for same-net
pads of neighbouring parts on one side, not constraints. 5x4_wave's 499
unconnected are its routing (25 of 28 connections open in both).

**Identity.** place-kicad-board with the short tier's prepared sources
and layout.json (build/short-x941, tasks v6), x990 against x1000:
byte-identical boards on spell_tome_bottom, USB_Cable_Tester, urchin,
eternal-keypad, OpenFC and Blue_Line. Short tier on x1000 (tasks v6, 2
jobs x 4 threads, under the lock, `build/short-x1000.log`) against
build/short-x941.log: compare.py same 12, unconnected 35 to 35, passes 6
to 6, new errors 12, vias 1129, length 22253 mm, work 2671 s, first-route
open terminals 61; every layout's placed board and result board
byte-identical. The only row field that differs is `constraints` on
MIDAS-MK1.1 (3 to 6), OpenFC (25 to 50) and reCamera (1 to 2): x941
(ad4362f) predates the placement room commit, which lists the tasks'
`rotation` entries (as x948 against x921).

Unit tests: `a_repeated_reference_entry_with_a_uuid_names_that_footprint`
(placement_constraints), the shifted cut and the starts in
`copies_of_a_tabbed_panel_keep_their_parts` (board_placer),
`global_placement_starts_parts_where_a_start_says` (pcb-placer tests).
Logs: build/panels-x1000.log, build/panels-ss-x990.log,
build/panels-ss-x1000.log. Snapshots x1000, x1001.

## Committed on 2026-10-15 (narrow reserve floor)

The layout ladder's reservation for the narrow-signal step
(`narrow_step_reserve`, "ladder reservation" above) was not a floor: a
rung, a finer pitch or a seed round started when the work left beyond
the reserve covered the last attempt's work, a finer pitch costs about
the square of the pitch ratio more, a rung's work deadline was the
pitch's whole work left (the reserve included), and the step then needed
only half of the reserve. Worse, the reserve's own floor (120 s) was
below the step's minimum (`narrow_step_minimum`, 180 s), so a board with
a short first attempt kept a reserve the step could never use:
TelemetryOnboard (v16 triage) kept 120 s, its three 0.05 mm rungs ran
(none better than the 0.075 mm tracks rung), 105 s were left, the step did not run and the
board ended 1 unconnected (v15 passed with the step).

Changed (crates/pcb-kicad/src/board_router.rs), only where the ladder
keeps a reserve (a work deadline, i.e. a layout, and a project where the
step applies):

- The reserve is at least `NARROW_RESERVE_FLOOR` (240 s: the step's
  180 s minimum and its clean-up at the negotiation's 60 s floor), at
  most the cap as before (0.4 of the work left, 600 s), and none when
  the cap is under the step's minimum (`narrow_step_reserve_floored`).
- A further rung starts only if its projected work leaves the whole
  reserve (`fits_before_reserve`; the last attempt's work, times the
  square of the pitch ratio on a finer lattice, `projected_attempt_work`,
  at least 120 s). The seed retries and seed rounds were already checked
  against the whole reserve.
- Work deadlines end where the reserve begins (`deadline_before_reserve`):
  a rung's at the pitch's work left less the reserve, each seed's at its
  share (seeds side by side count once each) of the work left less the
  reserve. A rung that stops at the deadline only does so where it would
  otherwise have eaten into the reserve.
- The step negotiates at least what the ladder's first attempt negotiated
  (its budget: the probe and its continuation, or the first rung's),
  bounded by the reserve and half of what is left
  (`narrow_step_negotiation_floored`).
- A board the ladder completes (a clean skeleton rung that goes on to
  the next rung) releases the reserve.
- The step still runs with half the reserve and at least 180 s left (a
  rung can still overrun its projection inside its deadline).

`PCB_LADDER_OLD_RESERVE` restores the old rules. Route mode keeps no
reserve and is unchanged, as are projects without the step and the
direct narrow-signal attempt of far-open boards (its configuration is
already narrow, so the step does not apply to it).

Layout runs, tasks v6, four threads, one run per side, unpinned; before
x941 (main at ad4362f), after x960 (this on 1223878; the polygon field
of 6aa79de changes no map; x961 is the commit rebased onto 6099e13,
whose neck repair acts only on boards with connection_width errors, none
of these). KiCad's unconnected and new errors, vias,
the layout's work (s); "same board" means the routed .kicad_pcb is byte
for byte the same:

| board | path | before | after |
| --- | --- | --- | --- |
| TelemetryOnboard | ladder, step applies | 1 / 0 / 102 / 1379, step skipped (105 s left of 120 kept) | **0 / 0 / 99 / 1196, passes**: 240 s kept, the 0.05 mm rungs did not start, the step ran (139 s of work, router 0 open) |
| MokyaLora | ladder, step applies | 0 / 0 / 333 / 1365, passes | same board, 1206 s: 240 s kept, the one-seed round of the best rung (1 open, about 160 s of work) did not start |
| A13 module | direct narrow attempt | 17 / 3 / 488 / 1653 | same board |
| vaio_re | direct narrow attempt | 15 / 0 / 716 / 1263 | same board |
| DSP-ADAU1452 | direct narrow attempt | 65 / 0 / 220 / 1276 | same board |
| Castor | no step | 0 / 0 / 167 / 924 | same board |
| keyboar_ | no step | 0 / 0 / 39 / 275 | same board |

No row was cut. module, vaio_re and DSP take the direct narrow-signal
attempt since x855 and never reach a ladder with a reserve, so this change
cannot touch them (their x941 numbers differ from x892's in the "after
the narrow attempt" table through the commits between, not this one).

DSP-ADAU1452 cold route (designer placement, no copper, route mode,
1200 s ladder, 4 threads): x940 1 unconnected, x960 the same board
(same attempts, 60 vias). It was not a reserve problem: route mode keeps
no reserve, and the step was not short of work. Its log (x903, x940,
x960): the best attempt before the step is the 0.075 mm tracks rung (20
open, 115 vias, ahead of the connect rung's 20 open and 135 vias on
vias); the step varies it, its negotiation reaches 1 net in conflict at
iteration 10, sits there to the price cap and stops on the stall at the
cap (`stall_at_cap` 12) after 25 iterations, with 16 s of negotiation
work of a budget of at least 60 s; the repair leaves 2 open. x817's step varied
the 0.075 mm connect rung (22 open against tracks' 24), completed in its
negotiation, and 96 of its 142 s of work were the via reduction of a
complete board. The 0.05 mm rung came after the best attempt was chosen
and only spent ladder work. What would help there is a step that tries
the other pour mode or the runner-up attempt when it stalls, not more
work.

Short tier x960 against build/short-x941.log (tasks v6): all 12 rows
identical (vias, length, work, KiCad's findings; 35 unconnected, 12 new
errors, 1129 vias, 2671 s of work; CPU 3913 to 3349 s is load). No
short-tier board keeps a reserve (no "kept for the narrow-signal step"
line in any layout log).

Unit tests: `the_floored_reserve_is_enough_for_the_step_to_run`,
`no_attempt_starts_into_the_reserve`,
`attempts_stop_where_the_reserve_begins`,
`the_step_negotiates_at_least_what_the_first_attempt_did_within_the_reserve`.
Logs: worktree pcb-maker-reserve2, build/runs (rows.py prints the rows
with the ladder's reserve, rungs and step lines; cold-x960 the cold
route), build/short-x960.log.

## Placement seeds after honest work (2026-10-15, x934 / x980, no code changed)

Question from the v16 triage (its item 9): since the placer's work is
counted honestly (93f40bd) and the pool runs only the seeds the budget keeps
(8435fff), the race runs fewer seeds (hackrf 16 to 10, WLED_Matrix 14 to
11, 4in1-mini 9 to 5, MIDAS-MK1.1 12 to 5 kept). Is the placement budget too
small? Answer: **no; noise or honest cost, nothing shipped.**

No setting exists for this: the layout sets the placer's `work_seconds` to
0.4 of `total_seconds` itself (`--placer` cannot change it) and the race's
first batch is `min(placement_seeds, 3)`. x980 = 1223878 plus two fields used
only for this measurement and not kept: the layout's `placement_share`
(0.4) and the placer's `minimum_seeds` (3), and a `--layout` JSON option in
run.py. Layout mode, tasks v6, four threads, three jobs, load 8-31, one run
per configuration (the layout is deterministic, no row cut):
(a) x934 as is; (b) `placement_share` 1.6 (2400 s of seed work, room for
16 seeds on all six); (c) `minimum_seeds` 16 (all 16 seeds in one batch,
whatever the budget); (d) as (c) with `congestion_probes` 6 (the race
probes six placements instead of three); and the designer's placement
(tracks stripped) in route mode on x934 as the ceiling. Logs and outputs
`build/seeds/{a-x934,b-x980,c-x980,d-x980,designer-x934}`.

KiCad unconnected (+ new errors); v15 and v16 are the sweeps' rows:

| board | v15 | v16 | (a) | (b) | (c) | (d) | designer, route mode |
| --- | --- | --- | --- | --- | --- | --- | --- |
| hackrf | 0 +1e | 2 | 2 | 2 | 2 | 0 +1e | 0 +1e |
| reCamera | 4 | 9 | 5 | 5 | 5 | 5 | 4 |
| WLED_Matrix | 4 +13e | 6 +1e | 6 +1e | 6 +1e | 6 +1e | 6 +1e | 1 +4e |
| 4in1-mini | 38 | 71 | 54 | 48 | 48 | 56 | 5 |
| Castor | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| MIDAS-MK1.1 | 11 | 9 | 8 | 8 | 8 | 8 | 8 |
| sum | 57 | 97 | 75 | 69 | 69 | 75 | 18 |

Vias 1416 / 1422 / 1422 / 1310 (a, b, c, d); CPU 7139 / 9811 / 8546 / 8251 s.

The placement per configuration (seeds kept of run, the placer's kept
seed; the clock's charge = the kept seeds' work as the reference pool of
eight threads runs them per batch; routing work left = 1500 s less the
charge; placement wall on four threads):

| board | (a) seeds, charge, left, wall | (c) | (b) | sum of seed work (a / 16 seeds) |
| --- | --- | --- | --- | --- |
| hackrf | 10/10 #4, 146, 1354, 214 | 16/16 #4, 153, 1347, 191 | 16/16, 196, 1304, 225 | 646 / 1048 |
| reCamera | 16/16 #1, 78, 1422, 60 | 54, 1446, 64 | 78, 1422, 82 | 402 |
| WLED_Matrix | 11/11 #2, 121, 1379, 168 | 122, 1378, 176 | 171, 1329, 308 | 603 / 877 |
| 4in1-mini | 5/5 #3, 297, 1203, 162 | 16/16 #16, 311, 1189, 400 | 443, 1057, 413 | 691 / 2100 |
| Castor | 16/16 #1, 28, 1472, 27 | 19, 1481, 39 | 28, 1472, 38 | 145 |
| MIDAS-MK1.1 | 5/7 #4, 309, 1191, 199 | 16/16 #11, 307, 1193, 341 | 406, 1094, 515 | 630 / 2082 |

What it says, per board:

- **The clock barely charges the extra seeds.** The charge is the
  reference pool's makespan per batch; (a) runs a batch of three and then
  one more batch, two rounds of the eight-thread pool, and (c)'s single
  batch of 16 is also two rounds. So 16 seeds cost the routing 0-14 s of
  its budget (less on reCamera and Castor, whose 3 + 13 batches become one),
  but 20 % more CPU over the six boards (+1400 s, 4in1-mini and MIDAS
  +1050 and +500) and twice the placement wall on four threads where the
  seeds are long (4in1-mini 162 to 400 s, MIDAS 199 to 341 s). (b) runs
  the same 16 seeds in 3 + 13 and charges up to 130 s more (4in1-mini 443,
  MIDAS 406) for the same boards as (c): the routing did not use it.
- **hackrf: not the seeds, the probe.** v15's final placement (RUDY 12.51,
  the best score) is one of (a)'s ten seeds too, and the race probes it
  first in every configuration (scores [12.51, 19.06, 19.6] in v15, v16,
  (a) and (c): the same three placements). v15's probes left [16, 16, 23]
  nets unfinished and it kept that placement (0 +1e); x862's and x934's
  probes leave [36, 72, 10] and keep the third (2). The probe is a 75 s
  route by today's router, so the router change since x723 moved the race,
  not the placer. With six probes (d) the fourth placement (RUDY 20.15,
  probe 0 unfinished) wins and ends 0 +1e, 603 vias against 701.
- **MIDAS-MK1.1:** 16 seeds bring v15's best-scored placement back (RUDY
  1.57, absent with five seeds); its probe leaves 24 unfinished against the
  kept one's 17, so the race keeps the same placement and the board is the
  same (8, 197 vias). v15's final placement is the placer's kept seed then
  and now (5 parts differ).
- **WLED_Matrix:** the extra seeds only change the third probe (4.29
  instead of 4.89, 119 unfinished); the same placement wins, identical
  board. v16's poses equal (a)'s; v15's (138 parts differ) are not among
  today's seeds.
- **reCamera, Castor:** 16 seeds already in (a); identical results in all
  configurations. reCamera's v16 row (9) is 5 on main: the router, not
  the placement (v16's triage counted it as placement).
- **4in1-mini:** seed 16 (beyond (a)'s five) scores best by RUDY and is
  kept: 54 to 48. With six probes (d) a placement whose probe left fewer
  nets unfinished (101 against 109) wins the race and ends worse (56):
  the probe's order does not hold for the final route when the counts are
  this close. v15's 38 came from placements today's placer no longer makes
  (every configuration differs from v15 in 188 of 191 parts; x792's far-side
  courtyards changed the seeds, Castor's sixteen seeds differ from v15's in
  162 parts too). The designer's placement routes to 5: the gap on this
  board is the placement itself, not the number of seeds.

Over the six boards: (b) and (c) 75 to 69 unconnected, all of it one board
(4in1-mini 54 to 48), with (d) giving 6 back on the same board; the
legaliser agent measured a placement change to flip crowded boards like
this one by far more, so one seed's difference on one board is not a
signal. More placement budget (b) costs routing budget (up to 130 s) and
CPU for nothing measurable; a floor of 16 seeds (c) is nearly free in the
clock but costs 20 % CPU and placement wall. Neither is clear enough for a
default, and the v16 "worse" rows were not seed counts: hackrf is the
probe race under the new router, reCamera the router, WLED_Matrix and
MIDAS unchanged with 16 seeds, 4in1-mini a placement gap that seeds do not
close (designer 5, ours 48-56). Short tier not run (no default changed).

Worth knowing for later: the race's probes (RUDY's best three, 75 s each)
pick the final placement by a margin smaller than their own noise on
hackrf and 4in1-mini (probe 36 for a placement that routes to 0-1; probe
101 for the one that ends 8 worse than the probe-109 one). The designer's
ceiling (18 unconnected over the six against 69-75) is in 4in1-mini (5 vs
48-56) and hackrf's vias (277 vs 603-701).

## Committed on 2026-10-15 (connection width)

KiCad's `connection_width` (drc_test_provider_connection_width.cpp, 10.0.7,
fetched from the 10.0.7 tag): runs only with a board-setup
`min_connection` (KiCad's default 0) or a custom rule, never from a
netclass. Per net and copper layer it unites zone fills (teardrops
included), tracks, flashed vias and flashed pads, fractures the union and
pairs each concave vertex with the nearest non-neighbouring vertex of the
same outline closer than the minimum less 2 x (DRC epsilon 0.0005 + max
error + extra fill margin 0.0005) = 0.115 mm for 0.127, seen from inside
at that vertex, with the outline bulging past the limit in x and y on both
sides. Vertex based and local: it also flags narrow gaps between two
parts of one outline ("gap" below). Position is not in the JSON.
`experiments/pour-fidelity/necks.py` reproduces it vertex for vertex on
kicad-cli's own fill (`--refill-zones --save-board`) and agrees with
KiCad on every board with findings (OpenESC's 199 are KiCad's cap).
Severity is the project's: error only on three corpus boards (CDFER
WLED_Matrix 0.127, USB_Keypad 0.127, USB_Cable_Tester 0.1); 15 others
have it as a warning.

Why our pour model misses them: kicad-cli's DRC with `--refill-zones`
first rebuilds teardrops for every pad with teardrops enabled (WLED: 3
zones become 636), and the necks are KiCad filler leftovers its
min-thickness pruning does not reach, all in hatched zones: the thermal
rings (added after pruning), the minimum-width border the iterative
refill restores around other nets' teardrop knockouts, pad corners
against hatch holes. Census (necks.py; build/necks/designer.txt,
results.txt, route.json, gaps.json):

| board | KiCad | kinds |
|---|---|---|
| designer WLED_Matrix | 4 | fill between pads / by a track (2 near other nets' teardrops), 2 gaps |
| designer USB_Keypad | 9 | fill near teardrops 5, thermal ring 2, by a track 2; 2 gaps |
| our WLED, v16 / short-x903 / short-x921 / short-x941 | 4 / 13 / 7 / 13 | GND F.Cu at the RGB LEDs' pad 3: pad copper or relief against hatch holes and the LED's own pad 1, a ring, fill near teardrops |
| our USB_Keypad, v16 / x903 / x921 / route x941 | 3 / 4 / 3 / 7 | GND F.Cu fill near teardrops, 3 of route's 7 are gaps |
| warnings: 0xCB pcb (ours) | 14 | thermal rings against other nets' copper |
| warnings: OpenESC 4in1, 4in1-mini (ours) | 199 cap (855, 1090) | our own tracks narrower than 0.16 |
| warnings: reCamera, MegaDrive, BAGEL, OpenESC designer | 1-30 | own tracks, a via, fill between pads |

Not modelled in the pour: it would need KiCad's teardrop generator and
its hatched iterative refill (polygon booleans the router does not have).
Built instead: a repair with KiCad's fill (`necks.rs`, on,
`PCB_KICAD_NO_NECK_REPAIR`; docs/router.md "KiCad's connection width").
When the final DRC has connection_width errors, kicad-cli refills and
saves a copy, the net's copper (fill outlines, pads, tracks, vias) is
rasterised at 4 microns, and saddles of the distance map (a pixel
narrower than the limit whose wider neighbours form two arcs, leading to
copper wider than the limit both ways within 0.5 mm) are the necks; on
the boards above it finds exactly KiCad's passages (7/7, 12/13 with one
duplicate pair, 3/3, 4/4). A track of the net, minimum wide, runs along
each into wide copper where it keeps class, hole, edge and other-fill
clearances (shifted or shortened if not); the board is checked again and
the tracks kept while connection_width drops and nothing else (error
types, unconnected) rises, by halves if all together fail (at most six
DRCs). Gaps are not repaired: the space's saddles were mostly gaps KiCad
does not report (Keypad: 20 candidates, 7 to 6 instead of 7 to 3). A
track that removes a Keypad neck sometimes adds a zone-zone unconnected
item in KiCad (not understood; the halving drops it). Cost: two to six
kicad-cli runs, 45-90 s wall, only on boards with such errors; a hook in
route_kicad_board's end and in the layout's end (a `necks.json` marker
stops a second try); `pcb-maker repair-kicad-necks <dir> <id>` runs it on
a routed directory; `board-router.json` gets `neck_repair`.

Numbers (x950; the commit's binary x951 is the same code plus comments):

- Route mode on the v16 placements, tasks v6, x941 against x950
  (build/necks-route-x9{41,50}.log): WLED_Matrix connection_width 4 to 0
  (5 tracks), USB_Keypad 7 to 3 (4 tracks; the 3 left are gaps),
  USB_Cable_Tester none; unconnected, vias, other errors identical.
- Short tier (flock, tasks v6) x941 against x950 (build/short-x941.log,
  build/short-x950.log): new errors 12 to 3, unconnected 35 to 35, vias
  1129 to 1129, passes 6 to 6, work 2671 to 2671 s, cut 0. Changed rows:
  WLED_Matrix connection_width 13 to 1 (12 tracks; new errors +11 to +2,
  the copper_edge_clearance it had), USB_Keypad 4 to 2 (2 tracks kept,
  under the designer's 9 either way). Every other row identical; the
  rows' length is the router's (the repair tracks, under a millimetre a
  board, are not in it). CPU WLED 609 to 637 s, Keypad 517 to 591 s (not
  a pinned pair).
- Starved-thermal census (census.py, fill3 KiCad cache, 44 boards) with
  x950: 162 predicted / 7 missed / 26 false alarms / 2 not modelled, as
  x912 (the router is untouched).
- Unit tests `a_thin_bridge_between_two_wide_parts_is_a_neck_a_wide_one_and_a_spike_are_not`,
  `findings_are_read_from_kicads_report`, `a_repair_track_keeps_its_clearances`.

Open: gaps (fill them along the slot, with KiCad's "same outline"
condition); the Keypad unconnected item; the WLED neck the repair leaves
(0.070 at a repair track, x950 short tier).

## Committed on 2026-10-15 (placement room)

The first three of the walkthrough's automation gaps (open problem 8):

- **A fast no.** Before placement (after the constraints are applied),
  a user's `row` that fits the outline's box in no orientation it may take
  and the parts held at one edge on one side that need more length than it
  has (single file: only parts too deep to stand two behind each other
  within their reach, each by its shortest tight body, the two that may
  overhang the ends by their copper) are input errors (exit 2), said
  together with the region errors (`placement_room::outline_holds`; not
  for an automatic outline, which the layout grows). In the placer,
  `hopeless` (computed on the last, loosest level before the levels run):
  the movable tight bodies need more area than a side's outline, or they
  cannot lie side by side in strips (`strips_hold`: bodies at least `t`
  across meet a line at most `floor(height / t)` times, so their least
  extents along it sum to at most that times the width). Both are exact,
  so a board that can be placed never takes this way; a hopeless one skips
  every level: one hurried legalisation from the global placement,
  `overfull` in board-placer.json and a hint. The work budget is checked
  inside the passes (`out_of_work`, a thread-local stop the running
  `place` sets): past it a part without a spot in `legalize` searches 31
  rings (15 mm of a 0.5 mm grid) instead of the board, `evict_for`,
  `refine`, `repair_relations` and `align_orientations` stop, the
  stuck-first retry is skipped.
  Urchin (walkthrough fixture) on 80 x 40 mm, four threads, load 20-35:
  with the walkthrough's constraints (rows, regions) x921 exit 1 after
  742 s (the "over 12 minutes"), now exit 2 in 0.04 s; move_all only on
  60 x 40 mm exit 1 in 3 s. Move_all only on 80 x 40 mm is not hopeless
  by either test (the KiCad 6 switches have no courtyard: their pads alone
  block, and pads could interleave), so it still takes the levels: x921
  398 s, x948 353 s (loads differ; the seeds' work 756 s against the
  600 s budget before, 604 s now: the passes stop at the budget).
- **Room as numbers** (`placement_room::room`, `room` in board-placer.json
  and in the verdict of place, layout and of an input error a constraint
  of the outline raised): per side the usable area (outline less the edge
  band and fixed bodies), the movable bodies with spacing, utilization,
  the area missing at a target of 0.7, the smallest outline at the target
  (area at the current aspect, then large parts side by side in strips as
  full as the target, choosing among sizes within a tenth of the least
  area the one nearest the current aspect; every part and row; edge sets)
  and, when parts did not fit, `suggested_outline_mm` with a next action.
  The urchin example (agents.md) suggests 118 x 107.5 mm; placed there,
  every part finds room (two near/relative entries written for halves
  side by side miss). An error verdict with room is written into a fresh
  output directory, and a directory holding only such a verdict is taken
  again by the retry.
- **What is kept.** `back`, `front`, `rotation` and `opening_outwards` are
  in `constraints[]` (`Constraints::kept`, `report_kept`; not in the
  placer's ranking, which sees only `report`). The verdict has
  `schematic_parity_by_type`, and a next action that says there is
  nothing to do when every finding is "No pad found for pin" (urchin's
  6 net_conflict, the controller's no-connect pins).

Identity (x948 = this on 140d0bf, against x921): place-kicad-board with
the short tier's layout.json and constraints (tasks v6) writes
byte-identical boards on spell_tome_bottom, USB_Cable_Tester, urchin,
eternal-keypad, OpenFC and Blue_Line; the short tier (tasks v6, 2 jobs x 2
threads, `build/short-x948.log`) against build/short-x921.log: every
layout's placement (placed/board-placer.json poses) and result board
byte-identical, all rows identical (compare.py same 12, unconnected 42
to 42, passes 6 to 6, vias, length, work, first-route open terminals)
except the `constraints` count on MIDAS-MK1.1 (3 to 6), OpenFC (25 to
50) and reCamera (1 to 2): their tasks' `rotation` entries are listed
now; nothing missed changed.
Snapshot `build/bin/pcb-maker-x948` (x940 was taken by another agent);
after the rebase on ad4362f, x949.

## Committed on 2026-10-15 (cold-route regression, x900-x903)

The two regressions of the Freerouting refresh, cold route (designer
placement, no copper), route mode, 4 threads, one run each (deterministic;
`build/cold/run.sh` and `table.py` in the worktree pcb-maker-cold).

Bisect, KiCad unconnected items (b399ec85 and b3c1f81d built from 399ec85
and 3c1f81d in a scratch worktree):

| binary | x155 | 399ec85 | 3c1f81d | x325 | x510 | x644 | x738 | x881 | x902 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 55 | 55 | 178 | 178 | 178 | 178 | 178 | 178 | 5 |
| MegaDrive | (95) | 95 | 137 | | | | 137 | 137 | 70 |

Both flip at 3c1f81d (2026-10-10, a rerouted net's corridor grows a tile
every sixth reroute, not every third). Mechanism (`PCB_ROUTER_DEBUG`, new
`route_net ...: search failed` and `resolve ...` lines):

- SNSP: GND (199 pads, four walled in at J3 and P6) failed a board-wide
  search in iterations 0 and 1 with those four open, so it kept to its
  corridors (`hopeless_searches` = 2, b72e61a); in iteration 2 a corridor
  missed and route_net stopped the whole net at 48 pads, frozen as
  blocked (21-48 pads for the rest), while the signals took the room.
  The repair ripped it up whole and its hard reroute from one pad reached
  21. Router and KiCad agreed all along (GND partial, 178 terminals): no
  counting bug, the 362/363 is per net. The narrower corridors only made
  the miss happen. Fix: pads a board-wide search did not reach are left
  out of the net's later searches in that negotiation
  (`unreachable_terminals`, `PCB_ROUTER_OLD_HOPELESS`).
- MegaDrive: narrower corridors miss more, the windowed fallbacks cost
  (open searches 228M expansions against 14M), the work budget ends
  negotiation at iteration 15 with 145 nets in conflict instead of 19
  with 106, and the repair ripped 77 nets up whole; 19 ended with no
  copper at all. Not a bug in 3c1f81d (it measured well on the quick
  tier), but the repair's whole rip-up is: it now removes only the
  conflicted branches (ignoring the negotiation's every-eighth whole
  rip-up) and keeps an incomplete net's tree (`resolve_keeps_partial`,
  `PCB_ROUTER_OLD_RESOLVE`).

Cold route, KiCad unconnected (x881 = main before; x900 = the pad rule
plus keeping incomplete trees; x901 + conflicted branches kept; x902 +
the stuck reset, = x903 before the rebase onto 670454c):

| board | x881 | x900 | x901 | x902 |
| --- | --- | --- | --- | --- |
| SNSP-CPU-01 | 178 | 5 | | 5 |
| MegaDrive | 137 | 137 | 147 | 70 |
| vaio_re | 8 | 41 | 5 | 5 |
| mackerel-30 | 0 | 0 | | 0 |
| DSP-ADAU1452 | 0 | | 0 | 1 |

vaio on x900: the tracks rung went 634 to 132 open and became the best,
and its narrow attempt ripped +3V3E up whole (3 of 43 pads): the reason
the repair change is part of this. DSP on x902: every rung as good or
better, but the ladder had time for the 0.05 mm rung (50 open) and left
the narrow-signal step 52 s of work instead of 142 (2 open against 0;
82 to 60 vias). SNSP CPU 816 to 283 s (one attempt, no seed rounds).

Short tier x903 against build/short-x817.log (tasks v6, current main):
unconnected 42 to 35 (MIDAS 26 to 20, WLED 8 to 6, reCamera 4 to 5),
passes 6 to 6, vias 1176 to 1129, new errors 6 to 12 (WLED_Matrix
connection_width 3 to 9: KiCad's GND fill necks at the RGB LEDs' pad 3,
on both sides, more of them on the new layout), work 2390 to 2677 s
(reCamera 65 to 366: its layout took another path; WLED 358 to 469),
CPU 4437 to 4865 s (not a pinned pair). Unit tests:
`a_walled_in_pad_does_not_stop_the_rest_of_its_net` (shows the old net
ending at the corridor miss), `the_hard_repair_keeps_the_branches_out_of_conflict`.

Quick tier (30 boards, tasks v6, 6 jobs), x817 (main at 670454c)
against x903 (this change on it): build/quick30-x817.log,
build/quick30-x903.log.

- x817: 24/30 pass, unconnected 27, new errors 2, vias 2913, length
  98215 mm, CPU 13555 s, work 9313 s, cut 0, first-route open 52; load
  median 17.9 (8.3-41.7).
- x903: 25/30 pass, unconnected 21, new errors 2, vias 2867, length
  99235 mm, CPU 13190 s, work 11453 s, cut 0, first-route open 47; load
  median 10.9 (1.7-34.7).

Better 3, same 27, worse 0; no row cut on either side; 23 rows
identical. The changed rows:

- OpenFC: 1 to 0 open, passes. Its first route 2 to 1 open; the ladder's
  regular rungs ran quicker, so the 0.075 mm pitch fit the refinement
  budget (x817: projected 296 s, over 240) and its re-seeded tracks rung
  finished (0 open, 169 vias against 159). Work 563 to 2143 s, CPU 464 to
  1802 s: the finer rungs really ran.
- MIDAS-MK1.1: 9 to 8 open, vias 240 to 197, length 3391 to 4464 mm (GND
  as tracks: the ladder's tracks rung now beat the first route, which
  x817 kept). Work 396 to 1322 s is accounting: the layout's work counts
  the ladder's only when its board is kept (src/agent_cli.rs), x817's
  ladder ran as long and was dropped; CPU 1572 to 1425.
- reCamera: 9 to 5 open, first route 10 to 8, work 891 to 610.
- WLED_Matrix: 6 to 6 open, vias 138 to 143, first route 10 to 8; errors
  the same (copper_edge_clearance 1; its connection_width necks are in
  the short tier's layout, not this one).
- USB_Keypad: 2 to 2, vias 99 to 78, work 513 to 445.
- urchin (work 97.3 to 93.8) and framework_mobo_lefthalf (391.8 to 392.0):
  same boards, the searches differ by a little.

New errors 2 on both (WLED copper_edge_clearance, eternal-keypad's), so
none came or went. Work +2140 s is OpenFC +1580 and MIDAS +926 (the
accounting above), less reCamera, Keypad, WLED.

The five cold boards on one binary pair, x817 against x903, KiCad
unconnected / vias / CPU s:

| board | x817 | x903 |
| --- | --- | --- |
| SNSP-CPU-01 | 178 / 1220 / 775 | 5 / 1456 / 302 |
| MegaDrive | 137 / 1148 / 615 | 70 / 1324 / 506 |
| vaio_re | 8 / 421 / 1142 | 5 / 369 / 961 |
| mackerel-30 | 0 / 46 / 574 | 0 / 46 / 555 |
| DSP-ADAU1452 | 0 / 81 / 1587 | 1 / 60 / 1450 |

(x903 identical to x902 on all five; x817 the same counts as x881 and
x738. x940, this change rebased onto f7addd8's zone seams: the same
counts, vias and lengths as x903 on all five.)

WLED_Matrix's connection_width findings (short tier): KiCad's own fill,
not ours (the routed board has no filled_polygon; the tracks rung leaves
GND's zone to KiCad's refill), necks of the GND zone on F.Cu at the
LEDs' pad 3 where other nets' tracks pass close. Not more GND as tracks:
GND track length 531 to 496 mm (F.Cu 409 to 423, B.Cu 122 to 73). The
x903 result is the tracks rung at the 0.075 mm pitch (10 open, the
regular pitch 14), x817's at the regular pitch (15; its 0.075 rung 23):
on the finer lattice other nets' tracks hug the LED pads, 25 to 31 of
the 64 LED GND pads with a foreign F.Cu track within 0.35 mm.

Not done: the walled-in pads could still be joined among themselves (a
forest of trees in the hard search: J3.5-J3.6, P6.5-P6.6 would save 2 of
SNSP's 5); a hard search that fails from the first pad's tree still ends
the net; the corridor schedule itself (3c1f81d) was not revisited.

## Committed on 2026-10-15 (zone seams)

The census's five TelemetryOnboard misses (J1/J2 SH3/SH4, D12.1, all on
F.Cu): its GND has four zones, a whole-board one of priority 0 (F, B), one
of priority 1 (F, B, In1, also the whole board) and two 4 mm bands along
the round edge, priority 10 (left, top) and 11 (right, bottom), drawn with
arcs. What KiCad 10.0.7 does (zone_filler.cpp, zone.cpp
BuildSmoothedPoly, drc_test_provider_zone_connections.cpp; the 10.0.7 tag,
not the master checkout in benchmarks/real/external/_downloads): each zone
is filled on its own over its outline and those of the same-net zones it
collides with (so reliefs and spokes are the same up to the seam), trimmed
to its own outline, and the outlines of the net's zones of a strictly
higher priority are cut out of it last (`subtractHigherPriorityZones`;
equal priorities overlap). The fills meet at the higher zone's outline
with no clearance between them. The DRC runs per zone and layer: a pad's
spokes are the crossings of its ring with that zone's fill, islands are
that zone's, tracks count where they end in that zone's fill, and a zone
whose fill does not cross the ring is no connection. The USB shield pads
(1.55 x 1.5 mm, 1 mm from the band's inner edge at y = 45.5) have their
ring 0.015 mm inside the band: three spokes in the priority-1 zone, one
(+y) in the band, "actual 1" for zone 11 (10 for J2.SH3). D12.1 sits in
the left band with its +x spoke crossing into the priority-1 zone: one.

Modelled (`zone_seams`, on, `PCB_ROUTER_NO_ZONE_SEAMS`): `Plane::above`
holds the outlines of the net's higher-priority zones on the layer; where
a net has two or more zones on a layer, `kicad_spokes` counts per zone the
pad's spokes whose ring crossing lies in the zone's part (sampled across
the spoke: a spoke the seam splits counts in both), its island spokes,
other pads' crossings and its tracks ending there, by the DRC's rule; the
worst zone's shortfall (`SpokeRecord::seam_short`, `seam_zone`) joins
`spokes_short`, the dump (`seam_short`) and spokes.py; a manual spoke for
it must end in that zone's part. Two fixes it needed: zone outlines drawn
with arcs are followed (`rule_area_polygon_points` dropped `arc`, so the
bands were polygons through their other corners; `PCB_KICAD_OLD_ZONE_ARCS`;
in the corpus only Telemetry's zones and a keepout of oro-os link have
arcs), and D12.1's +x spoke, which the model blocked: it ends in a strip
exactly the minimum width (0.2 mm) between its relief and D12.2's 0.2 mm
clearance, whose brush centres lie on one line 2 microns short of the
spoke's end (KiCad's end test is 5 microns past gap and half width); the
clearance test now runs from the ring to gap plus half width
(`spoke_clear_to_fill`, on, `PCB_ROUTER_OLD_SPOKE_CLEAR`). Not modelled:
per-zone fill settings (the layer's fill takes the last zone's clearance
and minimum width: Telemetry's bands keep 0.25, the priority-1 zone the
class's 0.2) and islands the seam alone makes (island verdicts stay the
net's one fill's).

Census (44 boards, KiCad side from the fill3 cache), x817 against x912
(x910 before the rebase, identical), and each change switched off:

| | x817 | x912 | no zone_seams | old spoke clear | old zone arcs |
|---|---|---|---|---|---|
| DRC starved predicted | 157 | 162 | 157 | 161 | 157 |
| missed | 12 | 7 | 12 | 8 | 12 |
| false alarms | 27 | 26 | 26 | 27 | 26 |
| starved pads not modelled | 2 | 2 | 2 | 2 | 2 |

Telemetry 7 found / 5 missed to 12 / 0 (D12.1 now as an island sliver in
zone 1, the strip the lattice misses: right verdict, KiCad counts it as a
plain spoke). The false alarm gone is Quanta75's FB2.1 (B.Cu, the same
end-of-spoke case); capybully's U1.8 changes one spoke from blocked to
none, verdict unchanged. Hub (several equal-priority GND zones per layer)
gets `seam` rows only on pads already short. No new false alarm.
Overlapping same-net zones of different priority (`experiments/
pour-fidelity/seams.py`, every census board and every v15 source and
result): only Telemetry in the census; in the corpus also OpenESC-20x20
4in1-mini (GND In2, priority 0 under 30) and SNSP-CPU-01 (GND B.Cu, 2
under 4), neither with a thermal pad near the seam.

Route mode on the v15 placements (tasks v6, `--jobs 1 --threads 4`, one
run per side), x817 against x910, `build/seams-x8{17,10}.log`:
Telemetry 108/108, 0 unconnected, 88 vias both, KiCad starved_thermal 5
(J1.SH3, J2.SH3/SH4, J1.SH4, D12.1, each "actual 1" in a band or the
priority-1 zone) to 0, length 3549.8 to 3559.4 mm (the manual spokes into
the short zones), other findings identical (the designer's 50 clearance,
22 solder_mask_bridge); 4in1-mini and SNSP-CPU-01 identical (53 and 259
unconnected, SNSP's 13 solder_mask_bridge and 2 starved on both).

Short tier x912 (rebased on 140d0bf) against build/short-x880.log (tasks
v6): all 12 boards byte-identical (no short-tier board has two zones of a
net on a layer, so only `spoke_clear_to_fill` could move them), 42
unconnected, 6 errors, 1176 vias, 22032 mm both; work 2390 to 2384 s
(framework_mobo 225.5 to 220.3 s, the same board: the shorter clearance
test asks the fill probe less). Tests:
`each_zone_counts_the_spokes_in_its_own_part` (router: a pad with three
spokes in the lower zone and one in the upper is short 1 in the upper, by
priority or overlapping at equal priority, 0 as one fill; the manual spoke
ends in the upper part) and
`a_zone_of_a_higher_priority_is_cut_from_the_nets_lower_zone_and_arcs_are_followed`
(lowering). Logs: build/census-seams (census x817.txt, x912.txt, the switched-off
runs try2-*.txt, seams.py's JSON), build/seams-x817.log,
build/seams-x910.log, build/short-x912.log.

## Committed on 2026-10-15 (agent walkthrough)

An agent's whole flow on two real boards, using only README.md,
docs/agents.md, docs/constraints.md and `--help`, from the KiCad project
with its schematic to a board `drc-kicad-board` passes. The harvested
GitHub boards carry no schematic (`benchmarks/real/external/github` holds
the board and project only), so the schematics and project libraries were
fetched at the boards' commits (origin.json): spell_tome_bottom (25 parts,
18 nets, 2 layers, its libraries in the project) and urchin (a split
keyboard, both halves on one board: 80 parts, 68 nets, KiCad 6 schematic
and libraries, MIT).

The flow (RAYON_NUM_THREADS=4):

```sh
pcb-maker import-kicad-netlist dl/spell_tome/spell_tome_bottom.kicad_sch tome spell_tome_bottom
pcb-maker describe-kicad-board tome spell_tome_bottom > board.json
# constraints.json: outline 70 x 100, U1 on the back, SW2 flush on the right
# edge opening outwards, J1 at the top, the four buttons in a region,
# L1 below U2 (relative), C2 near U2 pin 6
pcb-maker place-kicad-board tome spell_tome_bottom place1 tome/layout.json
pcb-maker layout-kicad-board tome spell_tome_bottom lay1 auto tome/layout.json
pcb-maker drc-kicad-board lay1/result spell_tome_bottom drc.json
pcb-maker export-kicad-fab lay1/result spell_tome_bottom fab
```

spell_tome: exit 0 in 41 s (73 s of work), 18/18, 9 vias, 586 mm, 8 of 8
constraints; drc-kicad-board 0 errors, 0 unconnected. urchin with 100
constraints (two 5 x 3 key grids as `row`s, a region per half, each of the
34 diodes `near` its key, thumb keys `relative` below the grid, the power
switches on the top edge, display and reset near the controller): exit 0
in 29 s (20 s of work), 68/68, 39 vias, 1897 mm, 100 of 100. With
`"total_seconds": 20` both finish in 12-13 s, still complete. Failure
paths tried: a 22 x 24 mm spell_tome (exit 1, `unplaced` U1, J1, J2 and
the hints in `next_actions`; 30 x 30 mm already routes clean), footprint
libraries removed (exit 1, `missing`), a netlist exported outside the
project, a file that is no netlist.

Friction found, and what was done:

- **Bug: a turned row was re-lowered the wrong way.** Layout mode lowers
  its own placement again for the report and the moves; `apply_rows` built
  the row from the leader's angle alone, so a row the placer had turned
  half way ran the other way: urchin's right half reported `region SW18`
  missed by 61 mm and `relative SW33 SW18` by 95 mm (exit 1 on a board
  that was fine), and the members would have been pulled there by the
  followers' sync. The row's frame is now the leader's angle or that angle
  turned by a half (a quarter with `turns`) turn, whichever matches where
  the members lie; a fresh stack matches all alike and keeps the leader's.
- **`near` could not name a key of a row** ("SW2 is carried by its row"):
  a keyboard's diode at its switch could not be said. `part_of` and
  `pin_of` may now name any row member: a new placer anchor
  `Anchor::Area(leader, member, centre, half)` is the member's body (or
  pad, half size 0) in the leader's frame, and the report names the member.
- **Bug: KiCad 6 library footprints kept `fp_text reference`.** KiCad
  reads it over the Reference property the import sets, so every copy of a
  footprint carried the library's reference (`Bat+0`): 181 schematic
  parity issues on urchin (73 duplicate, 73 missing footprints, 35 net
  conflicts). The import converts `fp_text reference` and `fp_text value`
  to properties (place, layer, font, hidden kept): 6 left, the controller's
  no-connect pins below.
- **276 false warnings "fixed parts X and Y overlap"** on every placement
  of a fresh import: the check ran before the constraints, when the stack
  was fixed by the default rules (parts off the board are on the edge);
  `move_all` frees them. It runs after the constraints now.
- **The layout's last line said `native complete=false`** above a verdict
  `complete` (KiCad's raw count includes the cosmetic silkscreen
  warnings). It now prints KiCad's unconnected count and its findings of
  any severity, "the verdict judges them"; agents.md says `routed.native`
  is raw and the verdict decides.
- **`describe-kicad-board` gave a fresh import a 3.8 x 0.8 mm outline**:
  SW2's slots on Edge.Cuts. `outline.from_footprints` is true when the
  board draws no Edge.Cuts of its own.
- **`place-kicad-board` needed a placer.json** that agents.md told the
  agent to cut out of layout.json; it now takes the layout config (its
  `placer` object) as well.
- **Missing footprints said only "not found"**: now where it was looked
  for (no library of that name, or the library's folder without that
  file), and the stderr line says to add the library and import again
  into the same directory (constraints.json and layout.json are kept; the
  help said "existing files kept", which read as "the board too").
- **A netlist exported outside the project** missed the project's
  libraries (9 of 25 footprints on spell_tome): the import also looks in
  the folder of the schematic the netlist names as its source.
- **A file that is no netlist imported as an empty board, exit 0**: now an
  input error (exit 2) that says how to export one.
- **`place-kicad-board` on a board without room did not end** ("checks
  constraints in seconds"): urchin on 80 x 40 mm ran over 12 minutes on
  two threads before it was killed; one part in a region too small for it
  took 55 s. The placer's relaxation levels anneal and legalize again and
  again, and the command had no work budget unless
  `PCB_PLACER_WORK_SECONDS` was set (with 10 s it still took 60 s of work:
  the budget is checked between levels only, and the first level's
  legalize, legalize-first and repair passes took 11, 10 and 8 s). The
  command now takes the layout's placement share by default (600 s of
  work, the same budget the layout would give it), and a `region` smaller
  than its part in every orientation it may take is an input error at
  once (exit 2, "region for Display1 cannot hold it: the box is 3.00 x
  3.00 mm, the part ...").
- **Help and docs**: layout's help said the unplaced case leaves a
  board-layout.json with only the verdict (there is none; verdict.json has
  it); `total_seconds` was only in `--help` (now in agents.md with what it
  does to the phases); the row rules (constraints on the first part hold
  the whole row) and two keyboard lines in the cookbook; import's help says
  what comes next and its exit codes; `unmatched_pins` on KiCad's
  `unconnected-(...)` nets are harmless but show as parity `net_conflict`.

Left as they are, small: every command's help repeats the agent exit-code
table, also for `describe-kicad-board` and `import-kicad-netlist` whose
codes differ; the import report's paths are relative where the verdict's
are absolute; `back`, `front` and `opening_outwards` are kept but not
listed in `constraints[]`; `drc-kicad-board` counts library findings
(`lib_footprint_issues`) as cosmetic that the layout's DRC does not run.

`tests/agent_walkthrough.rs` runs the documented flow on urchin
(`crates/pcb-kicad/tests/fixtures/urchin`: the schematic, project, library
table and the seven footprints used, with the MIT licence): import,
describe, a placement with a region too small for its part (exit 2 at
once), a placement that keeps all 100 constraints (a diode
reported against its key), the layout at `"total_seconds": 30` with
`PCB_ROUTER_NO_WALL_GUARD=1` (exit 0 and the verdict's fields), and
`drc-kicad-board` (exit 0). It is in the root package:
`cargo test --release -p pcb-maker --test agent_walkthrough` (14-28 s at a load of 20-25).

The row fix changes layouts whose row the placer turns (keyboards, LED
bars written as `row`); boards without rows place as before. The region
check and the placement default touch only `place-kicad-board` and
regions too small to hold their part. Snapshot: `build/bin/pcb-maker-x920`.
No benchmark tier was run.

## Triage of v16 (2026-10-15, x931 / x932)

The v16 sweep (x862, tasks v6, five jobs of four threads beside other
agents, load 16-35) against v15 (x723). The fix is on branch
v16-triage: x931 = x862's tree (068de8b) + fix, so that x931 against the
v16 rows changes only the fix; x932 = main f7addd8 + fix (tests, short
tier). Outputs were in the triage agent's scratchpad (not kept).

**1. The 17 cut rows.** Per row from the layout logs (seconds; "search"
and "pour" sum every phase's `profile work:` line, probes side by side
included, so they are thread sums, not wall; the KiCad column sums the
`board check` lines; lattice builds are the last `so far N lattices` count,
also summed over routers built side by side; the placement charge is its
work plus the pool's excess, `layout: placed`):

| board | wall | work | placement wall / charge | KiCad DRC (calls, hits, s) | lattice builds (n, s) | search wall / work | pour wall / work | cut |
|---|---|---|---|---|---|---|---|---|
| motherboard | 1702.5 | 909.5 | 262 / 122 | 2, 1, 16 | 7, 9.1 | 4159 / 2662 | 258 / 262 | deadline |
| link | 1709.0 | 1108.7 | 389 / 368 | 1, 1, 3 | 3, 22.3 | 3545 / 2353 | 280 / 304 | deadline |
| poly_kybd_split72_right | 1722.9 | 1232.9 | 131 / 192 | 1, 1, 2 | 3, 4.0 | 4120 / 2686 | 178 / 92 | deadline |
| poly_kybd_split72_left | 1800.7 | - | 194 / 210 | 0, 0, 0 | -, - | 3298 / 2084 | 146 / 80 | timeout |
| poly_corne_split42_right | 1784.9 | 1149.7 | 179 / 208 | 1, 0, 28 | 3, 5.9 | 3539 / 2193 | 191 / 103 | deadline,layout |
| poly_corne_split42_left | 1669.6 | 1168.4 | 96 / 125 | 1, 1, 1 | 3, 3.1 | 4182 / 2840 | 182 / 102 | deadline,layout |
| koeg-board-pcb | 1699.0 | 409.7 | 139 / 183 | 6, 5, 37 | 11, 40.1 | 1964 / 1050 | 42 / 54 | deadline |
| katia | 1671.9 | 585.6 | 200 / 323 | 1, 0, 17 | 3, 704.6 | 507 / 221 | 62 / 31 | layout |
| capybully | 1696.1 | 710.9 | 85 / 64 | 2, 0, 73 | 7, 179.1 | 1210 / 918 | 172 / 131 | deadline |
| Batch_1 | 1680.2 | 1325.7 | 370 / 387 | 4, 0, 225 | 6, 74.1 | 3432 / 3901 | 755 / 200 | deadline |
| Quanta75_BareRP2040_JLCPCBAo | 1721.3 | 1045.2 | 194 / 124 | 1, 0, 28 | 3, 42.8 | 2918 / 2175 | 243 / 243 | deadline,layout |
| woagboard | 1759.1 | 1201.7 | 89 / 76 | 11, 0, 115 | 19, 39.0 | 1833 / 1363 | 579 / 566 | deadline |
| panel | 1710.7 | 507.8 | 219 / 167 | 7, 0, 306 | 12, 567.3 | 1127 / 1765 | 640 / 293 | deadline |
| MegaDrive | 1800.7 | - | 130 / 253 | 0, 0, 0 | -, - | 4690 / 3537 | 89 / 85 | timeout |
| ghoul | 1700.7 | 1162.9 | 260 / 410 | 8, 0, 321 | 13, 44.9 | 1048 / 937 | 511 / 524 | deadline |
| ULK_sl_PG1316s | 1758.4 | 92.1 | 3 / 4 | 7, 0, 196 | 12, 1099.6 | 310 / 390 | 276 / 66 | deadline |
| ULK | 1800.6 | ~95 (at routed) | 1 / 2 | 8, 0, 201 | 13, 1201.4 | 335 / 406 | 313 / 63 | timeout |

No single phase explains them; four holes, by size:

- **Lattice builds of boards with a large cutout** (ULK, ULK_sl, katia,
  the 0xCB panel; 570-1200 s of the run). Not work, and not
  interruptible. Profiled (gdb samples, a per-obstacle trace): all of it in
  `StaticMaps::build`, `Shape::distance_to_point` of one board cutout
  measured for every node of its box edge by edge: ULK's 454-corner
  cutout over 16867 mm2, 15 s a class, four classes, 60-190 s a lattice
  (0.075 mm rungs the most); katia's 228-corner cutout 15 s a class; the
  panel's slots 0.7-2.7 s each. **Fixed** (`PolygonField` in grid.rs, a
  polygon of 16 corners or more): each edge updates only the nodes within
  reach of its box, the inside comes from each row's sorted crossings with
  `point_in_polygon`'s test; the distances are bit for bit
  `distance_to_point`'s below the reach (unit test against it on concave
  polygons with lattice-aligned corners), so the maps and the routes do
  not change. ULK's lattice 72 s to 1.2 s; route mode on v16's ULK
  placement, both measures in one binary (`PCB_ROUTER_OLD_POLYGON_DISTANCES`
  for the old): the 32 fingerprints the old side reached are all in the new
  side's (rungs side by side print in another order).
  Layouts on x931, one run each, four threads, load 6-21 (v16: 13-27):

  | board | v16 (x862) | x931 |
  | --- | --- | --- |
  | ULK | timeout at 1800 s, no row (v15: 66/70, 4 unconnected, 1 starved) | 1480 s, no cut, 66/70, 4 unconnected, no new error, 64 vias |
  | katia | 1672 s, cut (no final ladder), 139/162, 104 | 1382 s, no cut, the same board (139/162, 104, 401 vias); the final ladder ran and lost |
  | 0xCB panel | 1711 s, cut (deadline), 152/188, 86, +5 starved | 957 s, no cut, the same board (283 vias, 4594 mm); one more ladder rung ran and lost |
  | ULK_sl_PG1316s | 1758 s, cut, 68/70, 2 | 1710 s, still cut, the same board: its ladder now ran nine rungs (1200 s budget used) and seven uncached KiCad checks (22 s each), the last rung's seed retry was cut |

  The DRC cache hit the first route's and the early rungs' checks on
  ULK, ULK_sl and the panel: the boards were byte-identical to v16's.

- **KiCad DRC inside the layout is not work** (ghoul 8 calls 321 s, the
  panel 7 / 306 s, Batch_1 4 / 225 s, ULK 200 s, woagboard 11 / 115 s,
  capybully 73 s; an uncached check 10-50 s). A deterministic charge needs
  a model, and the obvious ones fail: over 57 v16 boards the median
  uncached check time does not follow the board's pads, tracks and vias
  or its file size (correlation 0.02 each); hatched zones (the panel,
  Batch_1, WLED_Matrix, capybully) and the load dominate. Counting the
  measured wall would make the layout follow the load. Not changed;
  options below. Done since: a charge per check from a constant and the
  hatch holes ("Committed on 2026-10-15 (KiCad check work)").
- **The search's work rate under the sweep's load.** On the boards that
  spend the whole budget (the four PolyKybd halves, link, motherboard,
  MegaDrive, Quanta75 Bare) negotiation and repair run 1.3-1.6 times
  their counted work in wall time: `EXPANSIONS_PER_SECOND` is an idle
  machine's, and five jobs of four threads already exceed the 5950X's 16
  physical cores, so with other agents (load 16-35 on 32 logical CPUs)
  most search threads share a core. The 10 % margin cannot hold there;
  this is the sweep's configuration as much as the accounting.
- **Pour analyses on hatched and big pour boards** run 2-4 times their
  modelled work (Batch_1 755 s of wall for 200 s of work, the panel 640 /
  293, PolyKybd 180 / 95; x862 still has the 41 ns constants, main
  recalibrated to 18 ns after the overhead round, so recheck on main).
- Smaller: the placement pool's charge (kept seeds at the reference eight
  threads) below the four-thread wall on motherboard (262 s of wall, 122
  charged) and Quanta75 Bare (194 / 124); above it on most boards. A
  layout's `work_seconds` in the row leaves out a final ladder it did not
  keep (koeg: 410 s reported, its unkept ladder ran 1100 s more of wall),
  so the rows understate the work that ran.

After the wall guard fires, the layout still runs what it does not
interrupt (lattice builds, repairs and stitching of the current attempt,
KiCad checks, labels and the final DRC), and the harness kills at 1.2x
(1800 s): that is how three of the cut rows became timeouts (ULK,
poly_kybd_split72_left, MegaDrive).

**2. New timeouts.** ULK: the lattice builds above (13 builds, 1201 s;
the first route needed 95 s of work); v15 finished in 1688 s because its
deadline was then the wall clock, which stopped the ladder; fixed by the
polygon field (row above). poly_kybd_split72_left: the first route's
negotiation ran 1046 s of wall for 677 s of search work (the load), the
wall guard cut it at 862 s of its work, and the two repair passes after
the cut (each about 60 s of negotiation, 300 s of hard repair and
stitching in all) ran past 1800 s; v15 (x723, wall-clock deadline) ended
at 396/420. Same class as the cut rows, no lattice or DRC share. 0xCB
pcb-panel: not a layout problem; the placement (306 s) and the first route
ran, then kicad-cli's zone refill timed out at 900 s (open problem 7,
`internal error: kicad-cli timed out`); v15 timed out at 1800 s before
getting there.

**3. The 13 worse rows** (compare.py counts unconnected only; PixelWave's
lost pass, one new error, is not among them). Placement against v15's
(poses of every footprint compared; v15's rows have no
`first_unconnected_terminals`), then the mechanism:

| board | v15 -> v16 (KiCad unconnected) | placement | route mode on v16's placement, x723 / x862 | x862 on v15's placement | verdict |
| --- | --- | --- | --- | --- | --- |
| Telemetry | 0 pass -> 1 | identical | 0 / 0 (88 vias both) | - | layout flow: the ladder kept 120 s for the narrow step, the step needs 180 s (`narrow_step_minimum`), 153 s were left, so it did not run (v15 won with it: 0 open; route mode's step: 0 open). Constants of 18a9a77: the reserve's floor (120 s) is below the step's minimum (180 s). Regression, on main too (not a far-open board) |
| ATAT1800 | 53 -> 54 | identical | - | - | noise: first route 67 -> 68 open, one attempt each, new errors 7 -> 4 |
| Explorer | 1 -> 2 | 2 parts moved | 3 / 3 | - | noise: first route 9 -> 3 open, the narrow attempt 8 open both |
| eurorack-pmod | 20 -> 21 | 3 parts moved | 22 / 17 | - | noise: router not worse; first route 19 -> 26, the polish kept the unpolished routes |
| protoesp | 83 -> 92 | 1 part moved, identical first route (88 open, 319 vias) | - | - | layout work: the first route and its resume booked 1246 s of work in 881 s of wall (parallel batches count each thread's expansions), so no ladder ran (v15, wall-clock deadline: four attempts). 30b14fb (x747) |
| module | 81 -> 82 | different | - | - | narrow-step reserve too small (154 s left of 319 kept); fixed on main by 1c8f17f / 974fa2b (x892: 17) |
| DSP-ADAU1452 | 105 -> 121 (+59e -> 0) | different | - | - | same; main x892: 82 |
| vaio_re | 101 -> 138 | different | - | - | same; main x892: 21 |
| BAGEL-MK1.1 | 0 pass -> 1 | different | 2 / 2 (identical) | 2 | noise in the layout path: router and placement give the same count in route mode |
| hackrf | 0 +1e -> 2 | different (205 of 421 moved, 0.635 mm steps) | 13 / 13 | 1 | placement |
| reCamera | 4 -> 9 | different | 9 / 9 | 4 | placement |
| WLED_Matrix | 4 +13e -> 6 +1e | different | 6 / 6 | 4 | placement (and errors 13 -> 1) |
| 4in1-mini | 38 -> 71 | different | 74 / 79 | 53 | placement (router 74 / 79 is noise) |

The router is not the cause on any of the eight boards routed in pairs
(x723 and x862 on v16's placement, route mode, four threads, one run
each: identical counts and vias but for eurorack and 4in1-mini, which
differ by 5 either way). Where the placement is the cause, the race ran
fewer seeds: hackrf 16 to 10, WLED_Matrix 14 to 11, 4in1-mini 9 to 5,
BAGEL-MK1.1 9 to 6. The placer's work now counts what the legaliser and
annealer do (93f40bd, x792) and the pool runs only the seeds the budget
keeps (8435fff), so the same placement budget holds fewer seeds; the
kept seed's wirelength is the same on these boards, the poses are not.
Suspects: 93f40bd and 8435fff; seed-repeated pairs would settle it (one
seed's result is not a placement's quality). Not regressions of the
code paths, but the budget now buys fewer seeds.

**4. Panels.** The rows changed wording more than behaviour: v15's
"placement is not legal" and v16's "incomplete: N parts without a legal
place" are the CLI exit-code commit's verdicts for the same failure.

- PolyKybd molecules (4x4, 4x5, 5x2_shifted, 7x5_wave_left; 5x4_wave
  places since x792's in-turn naming and routes 2/28): the parts named are the back connectors
  J5 and J6 (J9 on 7x5). The designer stacks them on the front J3 and J4
  (same spot, other side, SMD pads only, no courtyard) and KiCad finds
  no courtyard or other overlap there (DRC of the designer's 4x4: only
  starved thermals and cosmetic findings). What refuses them is the edge
  constraints of repeated references, named in turn since x792 (the k-th
  entry for J5 binds the k-th J5 in board order). The task's entries are
  not in board order, and a corner part's two edges are two entries that
  now bind two parts: placing the 4x4 with every part except J5 and J6
  fixed where the designer put them (`move_all` off) misses 17 edge
  constraints on the designer's own fixed J1, J3 and J4 (18.5, 37.6 and
  56.6 mm off: whole tile pitches) and leaves J5, J6, J2 and SW1
  without a legal place. So the rule is wrong for these tasks, not the
  geometry: a repeated reference's edge entries should be matched to the
  instances that can keep them (per entry, any instance; or the task
  generator writes them in board order with a corner's edges in one
  entry).
- SmartSpin2k_Panelized: `keep_copies` confines each KiKit copy's
  movable parts to its region (no edge constraints in the task, so the
  "region" of the hint is the copy's). The rule agrees with the designer:
  placing from the designer's poses (`move_all` off, x862) ends complete,
  every part placed and every constraint kept. From the prepared stack
  (every movable part at one point, inside one copy's region) 57 parts
  find no legal place: the search, not the rule. Start each confined part
  inside its own copy's region (its copy's fixed parts' centre) instead
  of at the common stack.
- OpenESC 4in1-panel: one net group (no copies), the front side 101 %
  full by the placer's bodies: crowding, as in v15 ("not legal" then).

**Ranked, what to assign next:**

1. Merge the polygon field (this branch; byte-identical maps, ULK no
   longer times out, katia and the panel no longer cut). Short tier x932
   against build/short-x912.log (main f7addd8 without the fix): all 12
   rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm, 2384 s
   of work); build/short-x932.log. Tests of pcb-router and pcb-kicad pass.
2. The sweep's configuration against the 1.1 margin: at most 16 search
   threads in all on this machine (four jobs of four, or three while
   other agents run), or a harness timeout and wall guard that scale
   with the measured load; otherwise every board that spends its budget
   is cut (8 of the 17).
3. Stop after the wall guard: once it fires, skip the remaining repairs,
   stitching passes and ladder rungs and go to the final check (three
   timeouts came from work after the guard).
4. KiCad checks in the layout: judge ladder rungs by the router's count
   and check only the candidates that could win, or charge a check from
   a per-board model of zone fill (hatched zones dominate; pads and tracks
   do not predict it), or store the check's seconds in the DRC cache and
   charge them on a hit as on a miss once known. Done: the gate
   ("wall guard stop, fewer layout checks") and the modelled charge
   ("KiCad check work").
5. Repeated-reference edge constraints (molecules): match entries to
   instances; then the four molecule panels can place.
6. Keep_copies start positions (SmartSpin2k).
7. Recheck the pour work model on hatched boards on main (2-4x under).
   Done: "Committed on 2026-10-15 (hatched pour work)".
8. The narrow-step reserve's floor (18a9a77): `narrow_step_reserve`
   keeps at least 120 s, `narrow_step_minimum` wants at least 180 s, so
   a board whose first attempt is short keeps a reserve the step can
   never use (Telemetry lost its pass to this; route mode's step
   completes it). Raise the floor to 180 s, or let the step run on what
   was kept. Done: "Committed on 2026-10-15 (narrow reserve floor)".
9. Placement seeds per budget (hackrf, reCamera, WLED_Matrix,
   4in1-mini): seed-repeated pairs of x723 and main's placer to see
   whether fewer seeds is the loss; if so, the placement budget in the
   new work units needs raising. Measured: not the loss ("Placement seeds
   after honest work").

## Layout sweep v16 (2026-10-15, x862)

docs/benchmarks.md "Layout sweep v16": 44 of 109 pass (v15 45), unconnected
3868 to 3527, new errors 289 to 82, better 13 / same 58 / worse 13, 17 rows
cut by the wall clock at load 16 to 35, which says a phase's wall runs well
ahead of its counted work on a quiet machine (KiCad DRC inside the layout
is not counted; the pool's charge is a reference 8-thread pool; candidates).
New timeouts ULK and poly_kybd_split72_left, the 0xCB pcb-panel ends on
kicad-cli's 900 s zone refill, SmartSpin2k and OpenESC panels stop with
parts the placer refuses. A triage agent (x930+) has the cut rows, the
timeouts, the worse rows and the panels; its section "Triage of v16"
follows. x862 predates the first-route budget (x855), hole clearance
(x881), router overhead (x872), pour recalibration (x817) and the second
narrow attempt (x893); the next sweep carries them.

## Committed on 2026-10-15 (after the narrow attempt)

The narrow-signal attempt run alone (first-route budget, above) did not
use the layout's work on every board: Hub ended at 577 of 1500 s, vaio_re
at 1005. What was left after it, in work (1500 less the layout's work and
the placement pool's excess), and what to run with it:

| board | left after the attempt (x855) |
| --- | --- |
| jetson (x881) | 114 s |
| A13 module | 40 s |
| DSP-ADAU1452 | 228 s |
| vaio_re | 410 s |
| Hub | 916 s |
| koeg | 53 s |

Changed (crates/pcb-kicad/src/board_layout.rs): after that attempt, with
at least 240 s left (`NARROW_CONTINUATION_FLOOR`), connections open and
the wall clock inside the layout's guard (`narrow_continuation`), the
attempt runs once more in the other pour mode (as tracks after connected,
and the other way round), negotiation and clean-up split from what is left
as for the attempt (`direct_narrow_budget`), that work its deadline; the
layout keeps the better board as before (open items, then starved
thermals). The work of the attempt and of this one now counts in the
layout's `work_seconds` whether kept or not (it used to count only a kept
board's; on the ordinary ladder fallback it still does). `skipped` says
when it did not run (with too little work: jetson, module, DSP, koeg).
`PCB_LAYOUT_NARROW_CONTINUATION` (stop, tracks, seeds, ladder) and
`PCB_LAYOUT_NARROW_CONTINUATION_FLOOR` are experiment switches; stop is
x855's behaviour. Unit tests for the decision and the seed count.

Candidates, one layout run each, four threads, unpinned, load 13-34, no
row cut. Unconnected and errors are KiCad's (new errors against the
designer's), then vias and the layout's work; in brackets the router's
open count of the extra attempt, kept or not. Before: x855 (jetson x881,
the hole-clearance fix; module, DSP and vaio_re rows from the first-route
agent's x855 runs, deterministic, same tasks file); candidates on x890
(this code on 1c8f17f, switches by environment); after: x892 (this code
with the default on 2b3b787, routes identical to x855 there).

| board | before (stop) | P: probe both pour modes for the attempt | T: other pour mode | S: fresh seeds | L: class-width ladder |
| --- | --- | --- | --- | --- | --- |
| jetson | 17 / 1 starved / 670 / 1307 | - | not run (114 s left); x892 identical | - | - |
| A13 module | 17 / 1 track_dangling / 490 / 1330 | 21 / 1 items_not_allowed / 409 / 1127 | not run (40 s); x892 identical | - | - |
| DSP-ADAU1452 | 82 / 0 / 225 / 1210 | 126 / 0 / 189 / 864 | not run (228 s); x892 identical; floor 180: 82 / 0 / 225 / 1441 (223, not kept) | floor 180: 82 / 0 / 225 / 1347 (one seed, 229) | floor 180: 82 / 0 / 225 / 1380 (339) |
| vaio_re | 35 / 0 / 686 / 1005 | not run | **21** / 0 / 637 / 1338 (55, kept); x892 the same | 35 / 0 / 686 / 1210 (one seed, 121) | 35 / 0 / 686 / 1310 (551) |
| Hub | 30 / 0 / 170 / 577 | 39 / 11 starved / 179 / 875 | 30 / 0 / 170 / 818 (244, not kept); x892 the same | 28 / 0 / 172 / 891 (4 seeds, 111, kept) | 30 / 0 / 187 / 1404 (114, kept: KiCad the same, 17 more vias) |
| koeg | 7 / 20 (15 courtyards, 5 shorting) / 427 / 1335 | not run | not run (53 s); x892 identical | - | - |

Steps (x892): jetson the attempt alone; module the attempt and a round of
3 seeds; DSP the attempt and 4 seeds; vaio_re the attempt, one seed, then
the tracks attempt; Hub the attempt with its 4 seed retries, then the
tracks attempt with its 4; koeg the attempt and 3 seeds.

What was learned:

- The other pour mode is the only continuation that gained: vaio_re 35 to
  21 unconnected with 49 fewer vias. On Hub it spent 240 s for a board it
  did not keep (Hub's leftover would otherwise go unused: CPU, not
  quality), on DSP (floor lowered to 180) 230 s for nothing.
- Fresh seeds: Hub 30 to 28 (two vias more, 314 s); on vaio_re one seed
  fit (255 s an attempt) and lost by 45 open; on DSP one seed lost too.
  One perturbed seed with less negotiation is a worse attempt, not a
  variation of the same one.
- The class-width ladder with 230 to 900 s: never better in KiCad's
  count (vaio_re: four probes and the continued rung in 306 s of the 410,
  551 open; Hub: 826 s for the same 30 and 17 more vias). The ladder needs the budget the first-route
  agent took from it, which is why the attempt beats it.
- Probing both pour modes (75 s each side by side, the better continued
  with the attempt's negotiation less the probes): tracks led every probe
  (module 60 against 123 unfinished nets, DSP 104 against 247, Hub 17
  against 78) and the continued tracks attempt then lost on all three
  (module 21 against 17 unconnected, DSP 126 against 82, Hub 39 and 11
  starved thermals against 30). A probe's unfinished count does not
  compare the modes: in connect mode it counts pour pads the pour does not
  reach yet, in tracks mode the pour nets are ordinary nets that finish
  late. The probe code was removed again; tracks as the second attempt,
  judged by KiCad, is the safe form of the mode choice.

Short tier x893 (this on 670454c) against build/short-x855.log: all 12
rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm, 2390 s of
work; CPU 5064 to 3924 s is main's router-overhead round). No short-tier
board has the narrow-signal step, so none takes this path.
Logs in the worktree's build/ (pcb-maker-narrow2: <board>-x855, -P,
-T, -S, -L, -T180, -S180, -L180, -x892, jetson-x881; rows2.py prints the
rows with the steps), short tier build/short-x893.log.

Left: on boards with little left (module 40 s, koeg 53 s, jetson 114 s,
DSP 228 s) nothing runs; a tracks attempt first instead of connect where
tracks wins (vaio_re) needs a predictor the probe is not. The ordinary
ladder fallback's work is still counted only when kept.

## Committed on 2026-10-15 (router overhead, x870 / x871 / x872)

Speed round four, unlike the three before it, changed no search: it looked
for CPU that buys no search and removed it with routes byte-identical.
Profiled route mode on the v15 placements of Castor, jetson-nano-baseboard
and k30-SBC (x862's code with debug symbols, four threads as the pairs run
them; `perf` is not installed and samply needs `perf_event_paranoid` 1, so
gperftools' `libprofiler.so` by `LD_PRELOAD` with `CPUPROFILE_USE_PID`,
200 Hz, libunwind stacks; tools and profiles in the worktree
`pcb-maker-speed`, `build/prof`). Share of the run's CPU (kicad-cli's DRC
included), Castor / jetson / k30:

| group | Castor | jetson | k30 |
|---|---|---|---|
| A* search proper (expansions, key heap, occupancy reads, heuristic) | 69.8 | 80.5 | 77.6 |
| the search's neck class lookup (`class_at` per step) | 2.2 | 1.2 | 0.0 |
| tile planner (`plan_corridor`: Dijkstra, `BinaryHeap`) | 3.0 | 5.9 | 8.8 |
| stamping and conflict bookkeeping | 6.8 | 3.0 | 4.1 |
| pour refresh and fill probes, negotiation | 3.3 | 0.5 | 3.5 |
| pour checks in clean-up and via reduction | 5.9 | 1.5 | 0.0 |
| lowering and lattices | 0.6 | 0.5 | 0.1 |
| KiCad (kicad-cli DRC) | 3.1 | 3.9 | 2.8 |
| allocation, memset, memcpy | 2.2 | 1.4 | 1.4 |
| SipHash | 1.8 | 0.4 | 0.8 |
| other | 1.3 | 1.1 | 0.8 |

Top symbols by self time (inlined callees included), Castor / jetson /
k30: `Router::search` 65.8 / 77.2 / 72.7 (its own lines: `KeyHeap::pop`
9.7 / 12.5 / 10.6, `occupancy_seen` 8.1 / 10.7 / 9.9), the search's
heuristic closure 4.6 / 3.6 / 5.6, `stamp`'s apply closure 4.9 / 1.8 /
2.9, `plan_corridor`'s entry-cost closure 1.4 / 2.8 / 3.9, `route_net`
(the planner inlined) 1.0 / 1.9 / 2.8, `PourMap::build` 2.6 / 0.6 / 1.0,
the planner's `BinaryHeap::pop` 0.7 / 1.4 / 2.2, `class_at` 2.6 / 1.4 /
0.0, `RandomState::hash_one` 1.9 / 0.4 / 0.8, `unstamp` 1.5 / 0.8 / 0.8,
`OwnStamps::contains` 1.8 / 0.4 / 0.9, SipHash `write` 1.1 / 0.3 / 0.4,
`Shape::distance_to_point` 1.1 / 0.0 / 0.2, `FillProbe::blocker` 0.6 /
0.0 / 0.1. The search proper was left alone (cheaper expansions were
exhausted in round two); the rest, with the share of the code each change
touched (Castor / jetson / k30):

- The pour analysis's own-stamp bitsets (`OwnStamps`) are found by a
  table indexed by map, not a SipHash map, asked once per node of the
  pour (4.0 / 0.9 / 1.7, most of it hashing).
- The search's neck class comes from the step's coordinates
  (`zone_class`), not `class_at` with a division per neighbour (1.4 /
  0.7 / 0).
- The tile planner computes each (layer, tile)'s entry cost once per plan
  (it was computed for each of up to eleven relaxations) and pops from
  the four-way `KeyHeap` with (cost bits, state) as one word, the order
  of the `(u32, u32)` pairs it popped before (3.0 / 5.9 / 8.7).
- A restamp builds the new list first (marks only) and then changes the
  counts only of stamps that went or came; the kept branches' stamps were
  decremented and incremented again on every reroute. The list, its
  order and the final counts and tile claims are as before (6.7 / 2.8 /
  4.0; `PCB_ROUTER_FINGERPRINT_ALL`, which hashes every occupancy map,
  identical). The diagonal footprint is mirrored in place (`turned`
  allocated a vector per diagonal node).
- `Board::fill_clearance` read `PCB_ROUTER_OLD_FILL_CLEARANCE` from the
  environment for every item a fill probe tests; now once (0.6 on
  Castor).
- The fill probe's index and hatch-hole memo hash with `FastHasher`
  (rustc's FxHash; iteration order unused) (0.4 / 0.1 / 0.2).
- `PourMap::build` looks its layer's maps up once, not per neighbour
  (2.1 / 0.5 / 0.8 touched).

After (a profile of all changes but the restamp, scaled to the same
search samples): Castor's run 11 % less CPU (own stamps and SipHash 3.5
points, planner 1.8, `class_at` 1.4, pour labelling and probes 1.5), k30's
9 % (planner 5.5, pour 1.8). Nothing in
the work accounting changed: the negotiation's limit is expansions and the
pour analyses count by their model (41 ns a node, 38 ns a stamp, 47 ns a
probe item). The model now overstates the analyses' wall time (Castor's
clean-up: 9.8 s of pour work in 6.7 s of wall, was 10.9 s wall), so the
budgets buy less pour checking than they could; recalibrating would move
results and wants its own measurement.

CPU, pinned simultaneous pairs in route mode (`--jobs 1 --threads 4`,
`PCB_MAKER_NO_CACHE=1` both sides, `cpu_seconds` of the rows; the base is
always main's code without this change; scripts `build/pairs/pair.sh`,
`run-pairs.sh`, `rows.py` in the worktree, rows under `build/pairs/r1`
to `r4`). The first side named ran on 0-7,16-23 except in r2:

| board | r1 x862 / x870 | r2 x870 / x862 (swapped) | r3 x855 / x871 | r4 x881 / x872 |
|---|---|---|---|---|
| Castor | 0.864 (511.0 / 441.3) | 0.915 (507.7 / 464.6) | 0.858 (561.8 / 482.2) | 0.862 (552.6 / 476.3) |
| k30-SBC | 0.888 (612.1 / 543.5) | 0.940 (524.8 / 493.4) | 0.939 (494.8 / 464.6) | 0.894 (644.0 / 576.0) |
| jetson | 0.895 (2462.6 / 2205.0) | 0.919 (1938.6 / 1780.7) | 0.878 (2236.4 / 1963.6) | 0.840 (2151.0 / 1807.3) |
| spell_tome | 0.960 (22.4 / 21.5) | | 0.995 (18.8 / 18.7) | 0.967 (18.0 / 17.4) |
| urchin | 0.958 (43.1 / 41.3) | | 0.891 (43.2 / 38.5) | 0.941 (37.6 / 35.4) |
| geomean | 0.912 | 0.925 (three boards) | 0.911 | 0.900 |

Ratios are new / old CPU. Over the four rounds (geomeans): Castor 0.87, k30 0.92,
jetson 0.88, the small boards 0.95 (KiCad's DRC more than routing:
spell_tome is 1.2 s of work in 18-22 s of CPU); the spread between rounds
is the machine's load (19-37 beside other agents). Every pair: the same
board file (md5), vias, length, work seconds and verdict; with
`PCB_ROUTER_FINGERPRINT=1` (r2, r4) the same fingerprints on every board
(jetson in r2 the same multiset: its side-by-side probes print in the
order they finish). During the work, `PCB_ROUTER_FINGERPRINT_ALL` (every
occupancy map hashed) at a fifth of the budget, two threads: Castor and
k30 identical.

Short tier (tasks v6) x871 against `build/short-x855.log`: all 12 rows
identical apart from CPU, wall, load and memory (vias, length, work,
unconnected, errors, first-route open terminals; compare.py: same 12,
unconnected 42 to 42, passes 6 to 6, cut 0 on both). The tier's CPU (5064
to 3185 s) is not a claim: unpaired, and the two runs saw different
loads. Rebased on the hole-clearance fix, x872 against
`build/short-x880.log`: again all 12 rows identical (compare.py same 12;
CPU 5216 to 4982 s, unpaired).

Next: the search proper is 70-80 % of route mode; outside it the largest
items left are the restamp's list building (the marks pass over every
branch on every reroute) and the clean-up's pour analyses (pour labelling
and the fill probe's distance tests).

## Freerouting refresh on x738 (2026-10-15, no code changed)

docs/benchmarks.md "Freerouting against our cold route, x738". Same picture
as the 2026-10-09 round (47 / 6 fewer unconnected, 51 / 8 clean where both
finish, 0.65x Freerouting's CPU), no board flipped sides. Our cold route
regressed on 11 boards since x136/x155: SNSP-CPU-01 55 to 178 open (all
GND; 362 of 363 connections "routed" while KiCad finds 178 unconnected
terminals: our connectivity and KiCad's disagree) and MegaDrive 95 to 137
repeat on a calm machine; katia hits the 1800 s wall at 14.9 GB. Assigned
to a cold-route regression agent (x900+). Fairness: Freerouting timeouts
above load 45 rerun, eight boards rerun Freerouting-only, 4 GB heap shown
harmless by A/B, keyboar_ is a parser hang (backtick in a part number), not
a timeout.

## Committed on 2026-10-15 (hole clearance)

Jetson's 8 hole_clearance findings (x855 layout) are one 3V3_SYS via at
(133.219, 122.186), 0.769 from the centre of J2's PTH pad 1 (GND; 0.75
copper, 0.55 drill). KiCad measures the via's copper (0.25 radius) to the
pad's hole (0.275 radius): 0.769 - 0.25 - 0.275 = 0.2438 where the
project's `min_hole_clearance` is 0.25; it lists the pair 8 times. To the
pad's copper the via kept 0.144 (0.125 needed), hole to hole 0.419 (0.25).
Not lattice rounding (the via sits on a 0.127 node, the pad off the
lattice; 0.775 needed, 0.75 kept) and not the narrow width: since b6e3530
a routed pad's plated hole was a hole obstacle of its net that blocked
other nets' tracks but not their vias ("vias stay with the pad copper
rule"), so vias kept only clearance plus copper, which with a 0.1 ring
and 0.125 clearance is 0.025 short of the hole clearance. The verifier
skipped it too (`blocks_vias` false), hence 0 internal violations.

Fix (crates/pcb-kicad/src/board_router.rs, crates/pcb-router/src/grid.rs):
the plated hole blocks vias; the static via map exempts the pad's own net
through `via_owner` where only the hole reaches (its copper still blocks
the own net as before). KiCad does not test a net's own copper against its
holes (every track to a THT pad ends on its drill; none of the 735
hole_clearance findings with a via or track in recent drc.json files pairs
items of one net). The verifier now checks other nets' vias against these
holes. Unit test `other_nets_vias_keep_the_hole_clearance_from_a_plated_hole`
(J2's geometry: lowering, static map for both nets, verifier 0.2439
against 0.25); it fails on x855 at the lowering and without the grid
change at the own-net exemption.

How common: the only finding of this kind in our routers' output. Of 3269
drc.json files of the last six days under build/ and the worktrees
(references and Freerouting left out), hole_clearance findings with a via,
track or zone are this via alone (jetson's x850, x851, x852 and x855
layouts). build/github-layout-v15 (100 boards) and v16 (39 so far): 20 and
9 boards with hole_clearance, every one footprint-intrinsic (pad to NPTH
of the same part, pad holes within J2, MX switch pins), none new against
the designer (v15's rows: no new hole_clearance).

Jetson in route mode, the a-x855 placement, the layout's narrow attempt
(`use_narrow_signals`, pours connect, no refine pitches, no retry seeds,
four threads; clean-up at its default 180 s, the 60 s cap is not a file
setting), three negotiation budgets per side since the result is chaotic
(identical up to iteration 48 at 405 s, where the via first differs):

| negotiation | x855 unconnected | x855 via findings | x880 unconnected | x880 via findings |
| --- | --- | --- | --- | --- |
| 380 s | 18 | 8 | 17 | 0 |
| 405 s | 11 | 8 | 16 | 0 |
| 430 s | 19 | 0 | 15 | 0 |

Sum 48 against 48; the other findings equal on all six (clearance 2,
courtyards_overlap 1, starved_thermal 4, the designer's 36
hole_clearance). Short tier x880 against build/short-x855.log (tasks v6):
all 12 rows identical (42 unconnected, 6 errors, 1176 vias, 22032 mm,
work 2390 s); none of its boards had a via in that ring.
Logs: build/repro in the hole-clearance worktree, build/short-x880.log.

## Committed on 2026-10-15 (first-route budget)

Jetson ended layout mode at 25 unconnected with no ladder at all. Where
its work went, x833 (four threads, `profile work:` lines, work seconds):

| phase | work |
| --- | --- |
| placement (longest seed; the pool's 79 s count toward the end only) | 52 |
| race: the two other placements' probes | 150 |
| first route: kept probe 75 + negotiation 478 (cap 556) | 553 |
| first route: hard repair 40, stitching and pour analyses 6 | 46 |
| move C33 (rejected, 39 open): negotiation 84, repair 37, stitching 3 | 124 |
| move C33 turned (kept, 30 open): negotiation 120, repair 31, stitching 2 | 152 |
| resume (82 s, 30 to 25 open): negotiation 83, repair 27, stitching 2 | 112 |
| clean-up, via reduction, spokes | 0 (polish needs 240 s, 218 left) |
| ladder | none (needs 240 s, 218 left) |

1203 s of work plus 79 s of pool: 1282 of 1500 (x832 before the pool:
1222 plus 46 of waves, wall-cut in the polish). The first route's
negotiation was not the problem (it stalled at 478 of 556 s); the moves
and the resume were: jetson's 31 open sat under the "many open" limit (5 %
of its terminals), so it was treated as nearly complete. On the boards
over that limit (module, DSP-ADAU1452, vaio_re) the ladder ran but its
probes and continued tracks rung took 900-1000 s, the reserved
narrow-signal step got 135-156 s or nothing, and no class-width attempt
beat the first route (x833: module 82, DSP 121, vaio_re 138).

Changed (crates/pcb-kicad/src/board_layout.rs): where the narrow-signal
step applies (`narrow_step_applies`), a board on four layers with pours
counts as far from complete above 2 % of its terminals (at least 10;
`far_open_limit`) as well as above the old 5 %; such a board skips the
moves, the resume and the polish, and instead of the ladder gets one
attempt of the step alone (`narrow_step_config`: signals at the narrowest
menu width, the minimum clearance where clearance findings are warnings;
the first route's pour mode, no finer pitches) with the work left: the
clean-up capped at a tenth of it (30-120 s, new `cleanup_seconds` in the
router config, not read from files), the negotiation three quarters of
the rest (60-900 s; `direct_narrow_budget`), its work deadline the
layout's end; the layout keeps it when KiCad finds it better.
`PCB_LAYOUT_OLD_FIRST_ROUTE_BUDGET` restores the old split. Unit tests
for the limits and the split. Boards without the step, and boards near
completion, take the old paths.

Layout runs, one per side, four threads each, unpinned, load 22-41;
before x833, after x855 (x855 is this on x862; x833 and x862 have the same
budget code, x862 changes hatched zones; the jetson, module, DSP and
vaio_re rows of x855 equal those of the x851/x854 runs built on x833).
Unconnected and errors are KiCad's (new errors against the designer's):

| board | unconnected | errors | vias | before: steps | after: steps |
| --- | --- | --- | --- | --- | --- |
| jetson | 25 -> 18 | 1 clearance -> 8 hole_clearance | 772 -> 710 | moves, resume, no polish, no ladder | narrow attempt (408 s negotiation, router 36 open) |
| A13 module | 82 -> 17 | 0 -> 1 track_dangling | 424 -> 490 | ladder, step skipped (154 s left of 319 kept) | narrow attempt (stalled at 103 s), a seed round |
| DSP-ADAU1452 | 121 -> 82 | 5 starved -> 0 | 186 -> 225 | ladder, step at 156 s: 258 | narrow attempt 169, four seeds 207 |
| vaio_re | 138 -> 35 | 0 / 0 | 551 -> 686 | ladder, step at 135 s: 530 | narrow attempt 76, one seed |
| Castor | 0 / 0 | 0 / 0 | 249 -> 167 | first route, moves, tracks rung | same; 167 is x862's (hatched zones; x862 alone gives 167) |
| keyboar_ | 0 / 0 | 0 / 0 | 39 / 39 | first route, moves | same |

No row was cut. Jetson's 8 hole_clearance findings are one via of
3V3_SYS 0.2438 mm from J2's PTH pad 1 hole (0.25 required; KiCad lists
it 8 times): a router hole-clearance rounding to look at, not the narrow
width. Two more boards with the step, on x833 against x854 with the same
rule (`PCB_LAYOUT_DIRECT_NARROW=always`, an experiment switch not kept):
Hub 30 -> 30, 187 -> 170 vias, 1378 -> 577 s of work; koeg 12 -> 7
unconnected, shorting_items 2 -> 5 (15 courtyard overlaps on both, both
runs wall-cut under load).

Variants measured on the way (jetson unless said): the narrow attempt with
half of what is left and the default 180 s clean-up (x850): 23, its
clean-up took 186 of 604 s; three quarters after a capped clean-up
(x851): 18, the negotiation still falling (21 nets in conflict); 0.85
(x852): 21, a plateau at 21 and another end; the step alone only when the
ladder could not afford probes plus two attempts of the first route's cost
(x851): module took it (17), DSP and vaio_re took the ladder and stayed at
121 and 138, which is why the rule now has no such threshold. A cap on the
first route itself was not needed on jetson (its negotiation stalled
under its cap) and would risk boards that complete in it.

Short tier x855 against x862 (same tasks, v6): all 12 rows identical, 42
unconnected, 6 errors, 1176 vias; against build/bench-short/short-x783.log
only 1337-v4.0's vias differ (64 -> 76, the recorded router change). No
short-tier board has the step, so none takes the new path. Logs in the
worktree's build/ (pcb-maker-jetson: six-x833, six-x851, a-x855, b-x855,
always-x854, extra-x833, extra-x854-always, jetson-x832, jetson-x852,
castor-x862), short tier build/short-x855.log.

Left: the narrow attempt does not use all the work on every board (Hub
ended at 577 of 1500 s): a class-width ladder after it with what is left;
a mode choice for the attempt (the first route's pour mode now; tracks
probed better on module, koeg and DSP); the via hole-clearance rounding
(not rounding: "hole clearance" below).

## Committed on 2026-10-15 (hatched zones in the tracks rung)

The tracks rung lowers its pours with `connect` false, and the fill probe
read a zone's hatch (and the outline edges) only from connecting pours, so
there hatched zones were modelled solid: the spoke verdicts, manual
spokes, island vias and stubs (only `add_inner_spokes` read the hatch,
through `hatch_any_plane`). The probe now reads every zone of the net as
KiCad fills it (`FillProbe::filled_planes`; `tracks_rung_fill`, on,
`PCB_ROUTER_OLD_TRACKS_FILL`): hatch, rings, hole grid and outline in
every rung. In the tracks rung the probe is used only after the routing
(the manual-spoke pass and its island repairs; the via reduction's starved
count is the corridor model), so the routing itself never saw the hatch.

Why reading the hatch alone hurt 0xCB (x860, route mode on v15: tracks
rung 46 vias, KiCad "2 spokes connected to isolated island" at MX3.S1 on
B.Cu): not the island rule and not the routing. MX3.S1 (a 2 mm PTH pad)
has two F.Cu tracks; KiCad's DRC counts a track connected to the pad that
starts in its ring and ends in this layer's fill, and one of them,
112.07 to 118.37 mm, ends in B.Cu fill. Our model tests a pad's straight
lattice run to its turn, which ran past that end (a junction: another GND
branch leaves it there, and `materialize` cuts the segment) to 98.77 mm,
outside the fill; the other run ends at 131.07 mm, 12 microns inside a
hatch hole (pcbnew agrees: not in fill). With the solid model the second
end was "in fill", so the model counted one track, found the pad short
with two island spokes, and added an island via; with the hatch it counted
no track, and with no track KiCad's rule lets the island spokes stand in
(spokes_short 0: no repair). KiCad counted the first track (1 counted, the
islands then ignored): starved. Fix: the DRC track rule cuts the run at
the first junction as the emitter does (`junction_nodes`;
`track_end_junctions`, on, `PCB_ROUTER_OLD_TRACK_RUNS`). The census
cannot see either change (it lowers with the pours connected and adopts
the board's exact segments). Router tests:
`the_tracks_rung_reads_a_hatched_zone_as_hatched`,
`a_track_ends_for_the_drc_at_a_junction`.

Hatched boards (`(fill ... (mode hatch`): 0xCB-1337 1337-v4.0, pcb, panel,
pcb-panel; Business-Cards Batch_1, USB_Keypad, WLED_Matrix; capybully;
le_chiffre stm32_chiffre_36keys and stm32_hotswap_chiffre; Qfwfq; the A13
module. Route mode on the v15 placements (tasks v6, `--jobs 1 --threads
4`), x832 (= main's router) against x861, `build/hatch/route-x8{32,61}.log`:
pcb-panel has no v15 placement and panel times out at 1800 s on both
sides; on the other 10 the same rung wins everywhere, unconnected 518 to
518, errors 14 to 14, starved thermals the same per board (Batch_1 7,
the rest 0), vias 1428 to 1427 (USB_Keypad 100 to 99), length 37025 to
37022 mm. 0xCB 1337-v4.0: tracks rung, 0 open, 0 starved, 47 vias on both,
MX3.S1's island via kept, and two manual spokes (B.Cu, four segments) the
hatched model no longer needs are gone (1953.2 to 1949.2 mm); capybully
-1.1 mm (two manual spokes fewer), Batch_1 +2.3 mm. Census of 44 boards:
x832, x860 and x862 identical (157 predicted, 12 missed, 27 false alarms,
2 not modelled), `build/hatch/census-x8*.txt`.

Short tier (tasks v6) x862 (this, rebased on main 9276fba) against
x833h (main's x833, run now as the base: short-x829 predates the work
accounting and the placement pool, which move 0xCB 1337-v4.0 to 76 vias
and 1611.6 mm on main itself), `build/short-x833h.log`,
`build/short-x862.log`: 10 of 12 boards byte-identical (placement and
result); USB_Keypad 100 to 99 vias, WLED_Matrix 2457.8 to 2459.2 mm (its
tracks rung adds 11 manual spokes, not 8; the model on x833h's board had
2 false alarms there), both 0 starved in KiCad on both sides; unconnected
42 to 42, errors 6 to 6, passes 6 to 6, vias 1177 to 1176, length 22030
to 22032 mm; OpenFC cut by the wall clock on both sides.

## Committed on 2026-10-15 (clean-up work accounting, x814)

The determinism agent left the clean-up's pour checks out of the work:
the pour analyses that keep a batch from stranding pour pads ran on wall
clock (Sisu: 21-47 s of pour checks in a 61-66 s budget; search work alone
covered 52-80 % of the clean-up's wall time), so a busy machine
stretched the clean-up and the route-mode guard (4 x the budget) could stop
it. Now every pour analysis counts as work in the phase budgets
(`budget_work` behind `clock` and `spent`: resolve, clean-up, via
reduction, thermal spokes): 41 ns per lattice node inside the net's pour,
38 ns per stamp of the net, 47 ns per item the fill probe indexes or looks
at (the spoke tests; `FillProbe` counts them). Calibration (route mode, one
thread, `PCB_ROUTER_POUR_TRACE`, seven boards): per board the model gives
0.89-1.07 of the analyses' wall time at load 17-22, 0.70-1.10 at 21-25
(x814, Castor and reCamera lowest); a lattice-only model spread 0.5-1.65
(the spoke tests are a third to two thirds of the time on reCamera and
Castor). At one thread the clean-up's counted work is now 0.98 of its wall
time on Castor (0.64 before). The verbose log has a `profile work:` line
(wall, search work, pour wall and work per phase) and the clean-up's last
line gives its work, its budget and the pour share. What stays uncounted
is in docs/router.md, "Determinism and the wall clock" (the negotiation's
stamping, the ladder's and the layout's tallies, `deadline_work`).

Route mode on v15 placements, x777 (main) against x814 side by side, two
jobs, load 29-35, one run each:

| | before | after |
|---|---|---|
| Sisu result | 7 unconnected, 408 vias, 5924.8 mm | identical (the winning attempt, signals at 0.1 mm on pours=tracks, has no pour checks) |
| Sisu clean-ups on the pour rungs | 404 / 308 / 234 analyses | 404 / 290 / 220; work 60.1 of 63, 69.3 of 66, 60.9 of 61 s in 80.9 / 80.5 / 61.8 s wall (work/wall 0.74-0.99) |
| Castor result | 0 unconnected, 3 starved (allowed 3, pass), 225 vias, 5186.2 mm | 0 unconnected, 4 starved (fail by one), 209 vias, 5156.1 mm |
| Castor clean-ups | first one finished its passes | each ends by work (60.1 of 60 s in 82.7 s wall, 60.3 of 60 in 59.6, 68.8 of 71 in 60.9) |

No wall guard fired on either side. Castor's change is the budget meaning
less work than before: the first clean-up stops earlier, the via
reduction starts elsewhere (16 vias fewer) and one pad more is starved;
a flip of the kind the short tier holds, not a fault of the accounting.

Short tier x814 (load 9 at the start, 7 at the end) against
build/bench-short/short-x783.log (both tasks v6): 12 of 12 the same by
compare.py (better 0, same 12, worse 0; unconnected 42 to 42, passes 6 to
6), 11 rows identical in vias, length and work. The one that moved,
1337-v4.0 (passes on both): vias 64 to 76, length 1594 to 1611 mm, work
169 to 130 s. Its first polish's via reduction (strict, 60 s) now counts
the pour analyses of its renegotiations and clean-ups, so after two rounds
it stops ("round 2 would take about 24s of the 15s left of 60s") where it
ran three (71 to 69 vias); the second polish starts from that board. Tests:
`cargo test --release -p pcb-router -p pcb-kicad` pass (new:
`pour_analyses_count_as_budget_work`). Logs: build/cleanup-x777,
build/cleanup-x814 (route mode), build/short-x814.log, calibration
build/cleanup-cal3-x813 and build/cleanup-val-x814.

Recalibrated 2026-10-15 (x817) after the router-overhead round (x872)
made the pour analysis about twice as fast (the old constants booked
Castor's clean-up 9.7 s of pour work for 5.2 s of pour checks, Sisu's
25.6 for 16.3): `POUR_NODE_NANOS` 41 to 18, `POUR_STAMP_NANOS` 38 to 26,
`PROBE_ITEM_NANOS` 47 to 42, fitted over three calibration runs at one
thread (load 15-31; build/cleanup-cal-x816, -x816b; the trace now also
prints the pour layers' nodes, which did not help the fit). Per board
0.71-1.08 of the analyses' wall time by the median, the big boards
(Castor, Sisu, reCamera, k30) 0.85-1.08; one board's ratio moves 0.6-1.2
between runs with the load, so 0.9-1.1 per board is beyond what one
calibration run can show. Route mode, x872 against x817 side by side (two
jobs, load 15-18): Sisu identical (7 unconnected, 408 vias, 5924.8 mm);
Castor back to its pre-accounting board (0 unconnected, 3 starved, pass,
225 vias, 5185.8 mm against x872's 209 vias, 5157.5 mm, also a pass now).
The clean-ups' pour work against their pour checks' wall time (four
threads): Castor 5.8 / 5.5 / 5.9 s for 5.1 / 4.3 / 4.9 s (x872: 9.7 / 9.0 /
9.6), Sisu 12.5 / 10.4 / 10.8 for 16.4 / 12.1 / 8.8 (x872: 25.6 / 20.1 /
19.0); each clean-up ends by work, no wall guard. Short tier x817 (load 17
to 10) against build/short-x880.log (tasks v6): all 12 rows identical
(vias, length, work, KiCad), 1337-v4.0 included.

Next: the ladder's and the layout's work tallies still count search
expansions only; counting the pour work there too (a `work()` in
`RoutingResult` instead of `expansions`) would make `deadline_work` and the
ladder's budget consistent with the phase budgets, at the price of shifting
more rows (every pour board's attempts), so it wants its own measurement.

## Committed on 2026-10-14 (placement pool)

The placement race ran its seeds in waves as wide as the machine's cores
(`available_parallelism`, 32 here whatever `RAYON_NUM_THREADS` said), each
wave as long as its slowest seed, and the waves' work counted toward the
layout's work end (`waves_work_seconds`). Now (`race_seeds` in
board_placer.rs) the seeds run in a pool of the process's search threads
(`RAYON_NUM_THREADS`), a seed starting as a thread frees; they are judged
in seed order as one after the other (seed k kept while the seeds before it
used less than the budget, three at least; ties to the earlier seed), as
the probe ladder judges its probes. The batches they start in are sized
from the kept seeds' work alone (the first holds the three, each later one
as many as the budget left holds at their mean work), so the seeds run and
the seed kept do not depend on the threads. A seed the budget would not
keep no longer runs: the waves ran one more seed after the budget was
spent and counted it (glasgow 260 s, the 0xCB pcb-panel 171 s, MIDAS-MK1.1
167 s of work for nothing). The clock counts the kept seeds as the
reference machine (eight threads, the calibration of
`EXPANSIONS_PER_SECOND`) runs them per batch (`pool_work_seconds`,
replacing `waves_work_seconds`); `board-placer.json` adds `seeds_run`,
`seed_kept` and `seeds_work_seconds` (their sum), and the layout's
`layout: placed` line prints them.

Six boards, one layout a side (x783 = main before, x830 = x783's tree plus
this; four threads, tasks v6, `PCB_ROUTER_NO_WALL_GUARD=1` on both so the
work alone decides, load 18-48; `build/pool/six-x7xx`). Wall and work in
seconds; work is the longest seed; the clock is what the layout counted:

| Board | placement wall | work | clock (waves / pool) | sum of kept seeds | kept / run | seed kept | final board |
| --- | --- | --- | --- | --- | --- | --- | --- |
| video | 421 / 457 | 439 | 567 / 567 | 1026 | 10 / 11 | 2 | identical, 263/371, 1238 vias |
| 0xCB pcb-panel | 471 / 314 | 170 | 491 / 320 | 652 | 4 / 4 | 4 | identical; both end on kicad-cli's 900 s zone refill (exit 4) |
| glasgow | 501 / 305 | 334 | 593 / 334 | 816 | 3 / 3 | 1 | identical, 112/226, 201 vias |
| Castor | 29 / 54 | 9.8 | 19 / 28 | 145 | 16 / 16 | 1 | identical, pass |
| USB_Keypad | 11 / 23 | 5.1 | 9.8 / 13.5 | 71 | 16 / 16 | 14 | identical |
| urchin | 8.8 / 13.8 | 4.7 | 8.4 / 10.7 | 55 | 16 / 16 | 6 | identical, pass |

The placements (poses) are identical on all six, so the kept seed is
x783's too; the result boards are byte-identical. glasgow's first route
negotiated with 467 s of work allowed instead of 337 s and took another
path (its fingerprints differ until the final ladder), but ended on the
same board. The placement's wall time follows the threads now: x783 ran up
to 13 seeds at once per board whatever the search threads, x830 four in
the tier, so boards of many short seeds place 1.5-3 times longer in wall
time (Castor 29 to 54 s, the same CPU); boards that ran a wasted seed are
faster. Placement alone with `RAYON_NUM_THREADS` 4, 8 and 16 on x830
(`build/pool/rayon`): seeds run, kept seed, clock and poses identical on
all six boards (walls 413/316/402 s video, 43/25/16 Castor, 308/286/274
the pcb-panel, 272/291/294 glasgow, 16/12/9 USB_Keypad, 10/7.5/6.4
urchin; the machine was loaded).

Short tier (x831 = main 6b3ced6 plus this, `build/short-x831.log`, against
`build/bench-short/short-x783.log`, both tasks v6): all 12 boards same,
every placement and every result board byte-identical; unconnected 42,
errors 6, vias 1165, length 22013 mm on both; work 2339 to 2428 s, CPU
4920 to 5127 s. The clock moved on every board (16-seed boards 1-5 s
more: the waves were 32 wide, the reference pool 8; less where a wave
waited on one slow seed or ran a wasted one: WLED_Matrix 177 to 113 s,
MIDAS-MK1.1 300 to 133 s, reCamera 75 to 52 s), and the work
left went into routing that did not win: WLED_Matrix's ladder ran its
two fine-pitch rungs (23 and 83 open against the kept 15; row work 282 to
358 s), MIDAS-MK1.1's first route negotiated 137 s instead of 53 s (334 to
347 s), reCamera's probe ladder ran where x783 had "deadline too near"
(its rungs lost). CPU: the pool runs the same seeds or fewer, so the
extra CPU is that routing; one pinned pair on reCamera
(`PCB_MAKER_NO_CACHE=1`, `build/pool/pair-x7xx`, load 40-58): 937 to 1061
s, the ladder's 4 probes and 3 rungs, identical row (55/56, 72 vias).
Tests: `placement_seeds_are_judged_in_seed_order_whatever_the_threads`
(threads 1-16, seeds finishing in reverse order, the sequential reference).

## Committed on 2026-10-14 (inner-pad spokes)

The exposed-pad model (above) found 0xCB-1337's two starved corner vias
but the router could not repair them: `manual_spokes` starts its tracks
from a pad's lattice nodes, and a via merged into the exposed pad has none.
`add_inner_spokes` (`inner_spokes`, on, `PCB_ROUTER_NO_INNER_SPOKES`; after
the manual spokes) gives such a via, where the spoke model finds it short
on a layer inside the exposed pad, a straight track of the net's width from
its centre into fill over the pad (in a hatched zone its own ring), its
copper wholly inside the exposed pad's, the shortest of sixteen directions,
kept only if the model then counts the via whole. KiCad's DRC counts a
track connected to the pad that starts in its ring and ends in fill as a
spoke; inside the exposed pad's copper the track keeps every clearance the
pad keeps. The tracks are exact copper (`NetState::inner_spokes`), counted
by the DRC track rule in `kicad_spokes`. Checked with kicad-cli on the
x825 board plus the two tracks before building it in: starved 2 to 0, no
other finding.

One trap: the tracks rung lowers pours with `connect` false, and the fill
probe read a zone's hatch only from connecting planes, so in that rung the
vias' rings did not exist and the model saw them unconnected (no finding,
no repair), while the ladder picked that rung's board. While
`add_inner_spokes` runs the probe now reads the hatch of any plane
(`Router::hatch_any_plane`); reading it in the tracks rung everywhere
changed 0xCB's tracks-rung routing (46 vias, MX3.S1 left an isolated
island), so it stays scoped. The tracks rung's fill model ignoring hatched
zones is a gap of its own.

Route mode on 0xCB's v15 placement (tasks v6), x825 against x828: the
same board plus two segments of 0.42 mm (length 1952.3 to 1953.2 mm),
starved_thermal 2 to 0, no new finding (annular_width 13 is the
footprint's vias), CPU 160 to 138 s. Census of 44 boards: x828 identical
to x825 (157 predicted, 12 missed, 27 false alarms, 2 not modelled). Short
tier (tasks v6) x829 against build/bench-short/short-x783.log: 11 boards
byte-identical, 0xCB-1337 the same board plus the two segments, its
starved_thermal 2 to 0 (it passed before on the designer's 3); passes 6 to
6, unconnected 42 to 42, vias 1165 to 1165, length 22013 to 22014 mm, CPU
4920 to 4951 s. An earlier tier against short-x791 was tasks v5 (the
list mix-up) and is void; WLED_Matrix on it was identical between x825
and x828. Router test:
`a_starved_thermal_via_inside_an_exposed_pad_gets_a_track_into_its_ring`.
Logs `build/fill3b-x8{25,27,28}.log`, `build/short-x829.log`.

## Committed on 2026-10-14 (ladder reservation)

The layout's ladder now keeps work for its last step, the best attempt
again with signals at the project's narrowest predefined width (plus the
minimum clearance when the project rates clearance findings as warnings),
on projects where that step applies (`narrow_step_applies`: 43 of the 109
v6 tasks have a narrower menu width or a relaxed clearance). In layout
mode the ladder's work end is about its budget, so the regular rungs used
to spend everything and the step never ran (module, DSP, Hub, vaio_re in
v15). Once the ladder's first attempt has run with connections open,
`narrow_reserve_share` (0.75) of that attempt's work, at least 120 s and
at most 0.4 of the ladder's work and 600 s, is held back: every later
rung, the seed retries and the seed rounds check the work left without it.
The step runs when half the reservation and at least 180 s are left (the
floor: on jetson, with 70 s left, a 60 s negotiation took 232 s and was
useless), negotiates a third of what is left with its router's work
deadline at what is left (it had no work budget when it varied a probed
rung, so it ran to the wall clock: DSP 619 open in v15), its work counts
in `work_seconds`, and `skipped` says why it did not run. Route mode and
projects without the step are unchanged (no reservation: the code paths
are gated on it). A limit on how many rungs run side by side was built
and removed: the rungs are judged in order either way, so it only
serialised waves and cost wall clock (Telemetry).

Layout runs, one per side, pinned (A 0-7,16-23, B 8-15,24-31, 4 threads
each); unconnected is KiCad's count, errors KiCad's; "step" = whether the
narrow step ran (router's open count after it):

| board | base / ours | unconnected | errors | vias | CPU ours/base | step base / ours |
| --- | --- | --- | --- | --- | --- | --- |
| A13 module | x747 / x772 | 81 -> 18 | 1 / 1 (pre-existing) | 414 / 404 | 0.86 | no / yes (37) |
| vaio_re | x769 / x773 | 163 -> 73 | 0 / 0 | 604 / 572 | 1.02 | no / yes (165) |
| DSP-ADAU1452 | x791 / x774 | 129 -> 108 | 0 / 5 starved thermals | 188 / 185 | 1.07 | no / yes (230) |
| Hub | x791 / x774 | 30 / 30 | 0 / 0 | 187 / 187 | 0.90 | no / yes (176, not kept) |
| MokyaLora (passes) | x791 / x774 | 0 / 0 | 0 / 0 | 334 / 334 | 0.96 | yes / yes (bounded: 157 s) |
| TelemetryOnboard | x791 / x774 | 1 -> 0 (pass) | 0 / 0 | 102 / 108 | 0.93 | no / yes (0); both wall-cut |
| jetson | x769 / x773 | 25 / 25, identical | 1 / 1 | 782 / 782 | 0.96 | no ladder (first route 1283 of 1500 s) |
| ghoul | x791 / x775 | 9 / 9, identical | 1 / 1 | 604 / 604 | 1.00 | step blocked: wall cut in placement |
| OpenFC (no step) | x791 / x775 | 1 / 1, identical fingerprints | 0 / 0 | 159 / 159 | 1.07 | - |
| protoesp (no step) | x791 / x775 | 92 / 92, identical | 2 / 2 | 295 / 295 | 1.00 | - |

The bases moved during the work (determinism, fill round two, exit codes);
each pair has both sides on the same base. Earlier pairs on x738 (before
the determinism change): module 81 -> 13, vaio_re 101 -> 21, Hub 35 -> 29,
DSP and koeg unchanged. Jetson cannot be helped by the ladder: its first
route takes the layout's budget (in route mode the step takes it from 48
to 6 open); a narrow first route when the project has the step is the
lever there, untested. Narrow is not a better default for the ladder's
first attempt: on designer placements (route-v12) the step beat the whole
class-width ladder on 9 of 16 boards (DSP 114 to 12, jetson 29 to 8,
vaio_re 36 to 16, Telemetry 8 to 0) and lost on 5 by 2-28 open, and in
layout v15 it lost on koeg (164 against 134) and Explorer (8 against 4).
The narrow step's DRC risk: the widths are the project's own menu (at or
above its minimum width), so KiCad raises no width finding; DSP's 5 new
errors are starved thermals of the narrow attempt's pours. Quick tier
x776 (this change on b2f5fbf, run on github-tasks-v6): no tier board
reserved or ran the step (none prints the reservation line). Its first
comparison, against build/quick30-x768.log, was not one: x768 ran on the
tier's default list then, github-tasks-v5, and v5 and v6 differ in the
constraints of 12 of the 30 boards (v6 fixes more parts: Castor J2 at the
designer's place instead of an edge constraint, BAGEL U102, USB_Keypad
and WLED_Matrix M1-M4, ...). Exactly those 12 rows changed (BAGEL 319 to
360 vias, Castor 230 to 249, USB_Keypad 81 to 100, WLED_Matrix's
connection_width 2 to 11; framework_mobo_lefthalf passes on v6); the 18
boards with the same constraints gave identical rows (1738 vias, 56520 mm
both). Castor swapped: x776 on v5 gives x768's row (230 vias, 4746.5 mm),
x768 on v6 gives x776's (249, 5601.1), same ladder and via-reduction lines
(`build/via-bisect/`). No commit between x768 and b2f5fbf changed routing:
the `spent` change of the exit codes only acts without a deadline (route
mode), the DRC cache only adds counters. The reservation's tier
comparison is x776 against a v6 base, build/quick30-x783.log (pending:
not finished when this was written). Logs `build/ladder-reserve/`
(summary.py there), `build/quick30-x776.log`.

## Committed on 2026-10-14 (placer fixes, x792)

Three placer defects from the v15 triage (`build/triage*`).

**1. The work clock under-counted the placer** (`placer phase` lines now
print the thread's CPU time; the ratio is work over CPU). The legaliser
booked 3-15 % of its CPU time on the 0xCB panels, the annealer 25-50 %
on boards with many relations, against 100 % on keyboar_, so the
layout's wall guard fired and results followed the load. Profiled with
callgrind (instructions per counted unit; about 42 on keyboar_'s anneal,
the reference) and fixed by counting what is done (details and the table
in docs/placer.md): cutout polygons per edge visited (the panels' slots
have thousands of points), box lists and pairs as the annealer counts
them, the other part's box and the linked lookup per same-side test,
relation anchors (glasgow's capacitors, 23 supply pins each), pin
positions (3 units), held parts' pad offsets, edge and region scans.
Exact speed-ups too: one pass over a cutout with a box prefilter (the
pcb-panel's legalisation 224 s to 28 s of CPU a pass), pads' edge offsets
only for parts an edge holds. Ratios, pairs on the two threads of a core
(before / after, whole placement): 0xCB pcb-panel 0.11 / 0.95, 0xCB panel
0.11 / 0.77, glasgow 0.40 / 1.08, vaio_re 0.72 / 0.98, MIDAS-MK2 0.52 /
1.07, Castor 0.68 / 0.98, urchin 0.86 / 1.08, keyboar_ 1.02 / 1.02;
phases now 0.63-1.6 (refine and repair on the panel up to 2.2, global
1.5-2.3 as before) where they were 0.03-1.5. A seed does exactly what it
did: only where budgets stop moves. The layout's placement step on the
tier's 30 prepared tasks (16 seeds, x738 against x780): 29 identical;
MIDAS-MK1.1 keeps 5 seeds instead of 12 (its seeds booked 0.45 of their
CPU time, now 1.05: twelve were 2.4 times the budget), wirelength 2119 to
2255 mm, the same 11 unconnected in the tier. Rebased on the determinism
agent's work (x781 = x747 + this): the waves' work now counts each wave's
slowest seed at its real cost too. Layouts, one run a side (x791 =
main against x792 = main + this, four threads each, load 16-49): 0xCB
pcb-panel, x791 never finished placing (1800 s timeout, cut); x792 placed
in 329 s (170 s of work, 321 s more in waves), routed (177 open after the
first route) and ended on kicad-cli's 900 s zone refill (open problem 7,
no KiCad count). 0xCB panel, x791 placed in 1248 s for 34 s of work and
was cut twice (router deadline, no final ladder), 136/188 routed, 213
unconnected; x792 placed in 222 s, cut once (a router deadline, 14 s of
its work done, load 30), 152/188, 86 unconnected. glasgow, both
identical (112/226, 263 unconnected, no cut), placement 755 to 544 s of
wall. vaio_re, no cut either side, 163 to 138 unconnected (8 seeds
kept instead of 11).

**2. Tabbed panels** (`keep_copies`, on): copies told apart by their
nets (KiKit's `Board_N-` names) and repeated references, each with fixed
parts apart from the others', keep their movable parts in their copy's
region (cut halfway between the copies' fixed parts). The 0xCB
pcb-panel and panel (four copies each) qualify; no tier board does. A
repeated reference in a constraint now names its parts in turn (the
molecules' J3 had a left, a top and a bottom edge at once).
Parts whose nearest fixed part belongs to another copy, the 0xCB panel:
13-20 in v14, v15 and on x791, 0 on x792; pcb-panel on x792: 2 (C47 and
C1, next to a cut line). The PolyKybd molecules share their net names across
copies (one net group), so the copy rule cannot see them; with the
reference fix 5x4_wave places legally, the other four still fail: the
designer stacked alternative connectors (J3 on J5, J4 on J6, pad on pad)
on the one spot per tile the fixed switches leave, which our rules
cannot reproduce. OpenESC's 4in1 panel is one net group too (its error
is crowding); the 20x20 4in1-mini is a 4-in-1 board, not a panel.

**3. Far-side courtyards of parts on one side** (`far_courtyard`, always
on): the legality test and the annealer keep one part's courtyard on the
other side from the other part's far side (holes, pads, courtyard).
woagboard's back-mounted choc switches: 28 courtyard overlaps in the
placement to 0 (KiCad DRC on the placed board). Not the cause elsewhere:
koeg-board's 15 are the designer's own (fixed switches against netless
mounting holes, both where the designer put them; the designer's board
does not load in kicad-cli, exit 3, so the runner cannot subtract them);
Quanta75's 45-66 were the runner's: its mounting holes are all "M2.5",
some on GND, so the GND ones were stacked and then held at the stack by
the netless ones' "M2.5" in `fixed`. run.py now leaves a netted
footprint in place when a netless one shares its reference (Quanta75
both 0 overlaps placed; it also changes WLED_Matrix's LDO1 and Hub's
REF**). Layouts on x792 (one run each): woagboard 26 courtyard
overlaps and 25 unconnected on x791 to 0 and 8 (one items_not_allowed;
both runs cut by the wall clock at load 20-50); Quanta75 Stamp passes
(0 overlaps, 162/162; v15 45 overlaps), Quanta75 Bare 0 overlaps (v15
66), 126 unconnected, 8 starved thermals (cut). The layout's verdict
could name the parts a repeated reference or a copy region held when a
placement fails (`unplaced`, `next_actions`); not done.

Quick tier x782 (x769 + this) against x768, both on the tier's own task
list (github-tasks-v5): 29 of 30 rows identical (routed, vias, length,
KiCad findings); WLED_Matrix 4 to 5 unconnected, from run.py's change
(its netted LDO1 now stays where the designer put it); 23 passes both,
unconnected 25 to 26, vias 2814 to 2824, CPU geomean 0.94, no cut rows.
The same binary's placements on github-tasks-v6 (the 30 tier boards
prepared once, x738 against x780): 29 identical, MIDAS-MK1.1 as above.
Logs `build/quick30-x782.log` (v5), `build/quick30-x781.log` (v6, 23 of
30; framework_mobo passes on v6's constraints, which fix M1).

## Committed on 2026-10-14 (exposed-pad thermal vias)

The largest class left by fill round two: 0xCB-1337's U3.45, ten
starved-thermal findings on six census boards the model had no row for
(thermal vias of the QFN's exposed pad, pads of its number merged into it,
which the model checked only where they are copper of their own). What
KiCad 10 does (zone_filler.cpp, drc_test_provider_zone_connections.cpp;
KiCad's filled polygons drawn per outline): every thermal pad
gets its relief, a hatched zone's ring (pad + gap to pad + gap + spoke
width, clipped by the zone outline only, then by other nets' clearances)
and four spokes from its centre, kept where their ends land in that fill
or meet another spoke, a pad inside a larger same-net pad included; the
thermal pads' plated holes are cut from the fill at the end. The DRC checks
each pad by its own ring half a gap out (crossings with fill outlines; an
outline over the larger pad holds both pads, no island) and its own
connected tracks (KiCad's connectivity: any track whose copper overlaps the
via, the exposed pad's tracks crossing it too). 0xCB's GND zone is hatched:
each via has a ring 0.4 to 0.7 mm and four spokes into it; the vias at the
exposed pad's corners lose their ring and outer spokes to the QFN pins'
clearances and keep the inward spoke: one, starved. In a solid zone the
larger pad's relief takes the vias' spokes and the DRC sees the larger
pad's own spokes crossing a via's ring (mackerel's mounting holes on In1:
2 on the spoke axes, 0 off them; solid there, so not in the census).

Modelled (`inner_pads`, on, `PCB_ROUTER_NO_INNER_PADS`): merged pads get
spoke records on every layer, reliefs and rings (`thermal_ring_pads`);
spokes of pads of one terminal meet; a landed spoke of a pad inside its
terminal's pad counts; foreign spokes across such a pad's ring are sampled
from their centre and count twice when they run through it; a merged pad's
DRC tracks are those overlapping its copper; no fill in a thermal pad's
plated hole. The earlier attempt's 15 false alarms came from the missing
rings (the vias' spokes found no fill), terminal-wide track ownership and
the track to the centre via's hole. Separately (`track_end_tolerance`, on,
`PCB_ROUTER_OLD_TRACK_ENDS`): the DRC's track-end test keeps other nets'
clearances to 0.5 microns instead of the spokes' 3 (`TRACK_END_TOLERANCE`);
four of the ten have a track from the corner via ending 1.5 microns outside
a sliver of fill between pins U3.23 and U3.24. Half a micron is KiCad's
DRC epsilon (`BOARD_DESIGN_SETTINGS::GetDRCEpsilon`, ADVANCED_CFG
`m_DRCEpsilon` 0.0005 mm, 500 nm read back in pcbnew), which its clearance
providers subtract from every clearance (`sub_e`); the zone-connection
check's point test uses none and the filler prunes with 1 micron, so the
number is KiCad's clearance tolerance, not that check's. Exact 12 missed /
29 false alarms, 0.5 microns 12 / 27, 1 micron 16 / 27, 3 microns 17 / 27.

Census, same tooling and KiCad cache (x801 = x820 = x768 before; x824
after, x823 before the rebase onto the DRC cache, identical):

| | x801 | x824 | x823 without inner_pads | x823 without track_end_tolerance |
|---|---|---|---|---|
| DRC starved predicted | 146 | 157 | 147 | 152 |
| missed | 13 | 12 | 12 | 17 |
| false alarms | 27 | 27 | 27 | 27 |
| starved pads not modelled | 12 | 2 | 12 | 2 |

By cause (`causes.py`, missed incl. not modelled): missing 12 to 2, tracks
4 to 3 (PixelWave's D45.4, the tolerance), the rest unchanged; false alarms
unchanged by cause. Every thermal pad inside a pad in the census (0xCB's 78
on six boards, Hub's U16.9_1/_2, Quanta75's U1.31; `pads_in_pads.py`,
mackerel's and PixelWave's are solid): 85 rows, 10 starved found, 75 agree,
none missed or false. Three router tests (hatched rings, the spoke through
a via's ring, merged pads on all layers).

Route mode on 0xCB's v15 placement, x801 against x824: identical boards
(47/47, 47 vias, 1952 mm, KiCad unconnected 0), starved_thermal 2 both (the
two corner vias, which the model now predicts), annular_width 13 (the
footprint's vias) both. The router does not repair them: `manual_spokes`
skips merged pads (no nodes of their own). A repair would be a short track
from the corner via inward into its own ring, which the DRC counts; not
done. Logs `build/census/x801.txt`, `x824*.txt` in the fill3 worktree;
`build/fill3-x801.log`, `build/fill3-x824.log`.

## Committed on 2026-10-14 (DRC cache)

KiCad's DRC (`kicad-cli pcb drc`, 6-40 s of wall clock, about 20 s of CPU
and 2.2 GB per call) is now cached on disk (`crates/pcb-kicad/src/drc_cache.rs`,
hooked into `native_report::run_kicad_report`, so every DRC of a route,
layout, verify-kicad-rung and the reference check goes through it; ERC is
about 1 s and is not cached). Key: SHA-256 (sha2 is in the tree) of the
kicad-cli version, the DRC arguments, the board's file name (the report's
`source`), every KiCad design file under the board's directory by relative
path and bytes (`.kicad_pcb`, `.kicad_pro`, `.kicad_dru`, `.kicad_sch`,
`.kicad_sym`, `.kicad_mod`, library tables; hidden entries skipped), the
global `fp-lib-table`/`sym-lib-table` and the `KICAD*` environment.
`kicad_common.json` is left out on purpose: kicad-cli rewrites it on every
run. KiCad's installed libraries are not hashed (they feed only the
`lib_footprint_*` warnings, which never count). Value: KiCad's report
bytes; a hit is validated like a fresh report and published with a rename.
Entries are `<key>.json` in `$PCB_MAKER_CACHE_DIR/drc`, else
`$XDG_CACHE_HOME/pcb-maker/drc`, else `~/.cache/pcb-maker/drc`;
`PCB_MAKER_NO_CACHE=1` turns it off; temporary file plus rename; least
recently used out beyond 2000 entries or 2 GB (a hit touches its entry);
any cache failure falls back to running KiCad. Visible as `drc_cache:
{calls, hits}` in verdict.json (and the reports' `verdict`) and on the
`board check:` stderr line, which also gives the lowering and lattice
totals so far. Snapshot x801.

Correctness (verify-kicad-rung, fresh with `PCB_MAKER_NO_CACHE=1`, then a
miss, then a hit, each in its own copy of the project): on Castor's routed
board, spell_tome, fence-esp32 (with schematic parity), jetson and Hub the
hit's drc.json is byte-identical to the miss's and verification.json is
identical across all three. Fresh against miss (two fresh runs) differs on
jetson and Hub beyond the date: KiCad's DRC is not byte-deterministic (the
order of findings, and which item pair names a missing connection); the
counts per type agree. One byte in the board (a footprint coordinate) and
one byte in the `.kicad_pro` each missed and added an entry on all five.
Unit tests: key derivation (every input changes it, unrelated and hidden
files do not, length-prefixed fields), atomic writes under eight
concurrent writers, eviction; an integration test with a fake kicad-cli
(two DRCs of one board: one KiCad run, one entry, identical bytes; a
changed byte or argument misses; a corrupt entry is dropped).

Savings, x801. Per call (verify-kicad-rung): Castor 5.8 s wall / 21.8 s CPU
to 1.1 s / 1.1 s (the rest is the preview SVG export, uncached), jetson
17-23 s to 6 s. Castor route mode on its v15 placement: 2 DRC calls (the
result and the reference), cold 557 s wall / 598 s CPU, warm 489 s / 578 s,
same board and drc.json; the board check's KiCad 38.3 s to 0.7 s. Short
tier (4 jobs, 2 threads, `build/drccache/short-{cold,warm}.log`): 54 DRC
calls, warm 53 hits (WLED_Matrix's cold run was cut by the deadline, the
warm one not, so its final board differs); 11 of 12 results identical;
tier wall 873 to 683 s, CPU 7008 to 5167 s (load-sensitive), and the
layout's peak memory drops from 2.1-2.3 GB to its own (0.4-1.4 GB) where
every DRC hits. run.py's designer DRC calls kicad-cli itself and is not
cached.

Within one run (cold cache) almost no DRC is on a byte-identical board:
spell_tome layout 1 call, Castor layout 2, Castor route 2, jetson route 5
(4 attempts and the reference), 0 hits each; the short tier 54 calls, 1
hit (reCamera) and 2 identical boards checked side by side (51 entries).
v15 layouts make 0-19 DRC calls (median 2). The cache pays across runs:
reruns, A/B pairs on the same placement, the reference board.

Follow-up (x802): `pcb-maker drc-kicad-board <project-dir> <board-id>
<report.json> [--no-refill]` runs the cached DRC on the board and its
rules as the reference check does (`pcb_kicad::kicad_drc_report`; exit 0
no error that counts and nothing unconnected, 1 otherwise, 2 input, 4
KiCad failed; `tests/cli_exit_codes.rs` with a fake kicad-cli). run.py's
designer check and freerouting.py's designer and Freerouting-result checks
use it when `<binary> --help` lists it (probed once per binary; older
snapshots keep calling kicad-cli). Its report has every severity; the
designer counts (errors only, not excluded, not cosmetic) equal the old
`--severity-error` counts on qfwfq (546, at KiCad's 199 cap), WLED_Matrix,
USB_Keypad, Castor, 1337-v4.0 and MIDAS-MK1.1. run.py rows gain
`designer_drc_seconds`. Short-tier splice of spell_tome and Blue_Line
twice: the designer DRC 7.2 s to 0.4 s and 4.6 s to 0.4 s, the layouts'
own DRC hits too (CPU 100.5 to 91.9 s and 53.8 to 44.5 s), same results.

Lowering and lattice building (the second target): measured, not cached.
jetson route 7 lowerings 1.3 s and 7 router builds 15.4 s (summed over
threads) of 1558 s wall; Castor route 4 builds 4.3 s of 557 s; Castor
layout 22 lowerings 0.1 s, 4 builds 2.6 s of 904 s. Under 1.1 % everywhere,
below the 5 % bar.

## Committed on 2026-10-14 (benchmark tooling: short tier, route mode, splicing)

Florian's feedback: we spent most of the day waiting for CPU, because every
agent ended with a serialised full-budget 30-board tier and paired every
measurement. `benchmarks/agent-tasks/README.md` "How to measure" now states
the discipline: the short tier (`--tier short`, 12 flip-prone boards from
`flippers.py` over x644..x782, a third of the budget via `--budget-scale`,
about 600 s at 6 jobs; WLED_Matrix gets wall-cut above load 45 at that
budget) for iteration; the 30-board tier at full budget only as the merge
gate; quality judged by one run per side (the layout is deterministic unless
`cut`); pinned simultaneous pairs only for CPU claims; router questions in
route mode on fixed placements (`--route-from <sweep dir>`, verified
identical to manual routes on two boards); incremental sweeps with
`--splice-from <log>` (rows marked `spliced`, compare.py shows SPL); an
objectives line (unconnected, new errors, vias, length, CPU, work, cuts,
first-route open terminals) in the summary and summary.json.

## Committed on 2026-10-14 (CLI exit codes)

A benchmark agent found `route-kicad-board` exiting 1 on most harvested
boards with 0 open connections: the CLI failed on any error-severity DRC
finding, the designer's own included (USB_Cable_Tester on its v15
placement, x782: 54/54 routed, KiCad 0 unconnected, one `courtyards_overlap`
H4/J2 that the designer's board has too; Castor: 126/126, KiCad 0
unconnected, 4 `solder_mask_bridge` and 3 `starved_thermal`, exactly the
designer's board's, plus a cosmetic `footprint_type_mismatch`). The
layout's exit code used the router's count instead (pads joined through a
pour's fill counted open), and every error, a missing file and a panic
alike, exited 1 (a panic 101).

The contract now (`pcb-maker --help`, docs/agents.md section 4):

| Exit | Status | When |
| --- | --- | --- |
| 0 | complete | KiCad: no unconnected item, no error beyond the reference board's per type (cosmetic types and warnings never count); layouts and placements also place every part and keep every constraint |
| 1 | incomplete | finished, result written, but not complete |
| 2 | usage_error | arguments, missing or invalid inputs, unknown keys or parts (`pcb_kicad::input_error` marks the library's input-stage errors), output directory exists (now checked before the run, not after an hour of routing) |
| 3 | cut | incomplete and `cut` non-empty (a complete result with a cut is 0) |
| 4 | internal_error | other errors and panics (one stderr line; the panic hook keeps the default report only with `RUST_BACKTRACE`) |

The rule is run.py's: KiCad decides, not the router's count; the
reference's errors are subtracted per type (`pcb_kicad::verdict`, unit
tests there). The reference is `--reference <project-dir>`: route mode
defaults to its input as given (for a harvested board, the designer's
routed board, which is run.py's `designer_findings`), layout mode to none
(its input is stacked parts); it is checked only when the result has errors
that count (one more KiCad DRC, `reference-drc.json`). Schematic parity is
reported, not judged (run.py does not judge it either; the old route check
did).

Every route, layout and placement writes `<out>/verdict.json` (also on an
error once the output directory exists, and for a layout whose parts found
no place, which writes no board-layout.json, so that scripts' "report
missing" test still means "no layout") and the same as `verdict` in its
report: status, exit code, one-line reason, router and KiCad open counts,
new and pre-existing findings by type, cosmetic and warning counts, cut,
work and wall seconds, absolute output paths, skipped ladder steps and
next actions. board-router.json gained `work_seconds` (the ladder's work)
and `skipped` (the narrow-signal and minimum-clearance steps, finer
lattices and further rungs that did not run while connections were open,
with why). run.py records `exit` and `verdict` per row and takes a missing
report's error from verdict.json; its pass rule is unchanged. freerouting.py
and the corpus and PCBench runners read only the reports; they are
unchanged (their route runs now pay the reference DRC on boards whose
result has errors: about 10-50 s of wall clock, not in `routing_seconds`).

Found on the way: every route-mode run that skipped the probe ladder
reported `cut: ["ladder"]` (x746 to x782): the ladder's `spent` noted a
cut whenever the wall clock over 4 was ahead of the work, which at the
ladder's start (0 s of work) is always. The guard now takes over only when
the wall clock is ahead by more than a twentieth of the ladder's budget
(60 s of work by default); below that the shares are the work's alone.
USB_Cable_Tester routes the same board (86 vias, 2292.4 mm).

Verified: USB_Cable_Tester route on its v15 placement, exit 1 (x782) to 0
(x790), the courtyard overlap pre-existing; Castor (wntrblm mainboard, v15 placement) exit 1
to 0 with the same board as x782 (126/126, 225 vias, 5186.2 mm), its 4
solder_mask_bridge and 3 starved_thermal pre-existing in the input as in
the designer's board, 419 s wall for 399 s of work, the reference DRC
included; the tiny
three-pad fixture in route, place and layout mode (complete, unplaced parts,
a missed `near`), usage errors and a forced panic
(`tests/cli_exit_codes.rs`, `PCB_MAKER_TEST_PANIC`).

Left for agents to work around: a process killed by a signal (a harness's
timeout, the OOM killer) has no verdict; `import-kicad-netlist` now exits 1
when footprints are missing; research commands keep 0/1/2 (+4 on a panic)
without the incomplete/internal distinction. Tests: pcb-kicad, pcb-router,
pcb-placer and tests/cli_exit_codes.rs pass. Three tests of the root
crate's tests/kicad_ladder.rs fail, before and after this change (the
research ladder, code this change does not touch; a 2026-09-30 test build
fails the first one too): native_one_connection_transaction_commits_or_exactly_rolls_back,
native_shared_tree_insertion_reuses_copper_without_a_via_cluster and
trace_tension_can_move_a_real_rung_one_footprint_without_breaking_kicad.
A closed stdout (`pcb-maker --help | head`) ends quietly with exit 0 (or
the verdict's code when a run already had one).

## Committed on 2026-10-14 (fill round two, x768 / x769)

The census of 44 boards (`experiments/pour-fidelity/census.py`, KiCad's
starved-thermal findings against our pour model) before and after, same
tooling (the tooling now reads standard layer names; rosco's In1.Cu is
called "Route2"):

| | x760 (= x738 model) | x768 |
|---|---|---|
| DRC starved findings predicted | 129 | 146 |
| missed | 28 | 13 |
| false alarms | 90 | 27 |
| starved pads not modelled | 14 | 12 |

What changed, all on by default, each with the census step it bought (env
switches for A/B): track ends found anywhere across a trimmed spoke; spokes
meet at any angle; pad clearance overrides replace the zone clearance in the
fill, also when smaller (`Board::fill_clearance`, `PCB_ROUTER_OLD_FILL_CLEARANCE`);
sliver ends; spokes in the pad's own frame (`Board::pad_spokes`,
`PCB_ROUTER_OLD_SPOKE_FRAMES`; the routing guards keep the box axes, frames
there cost Hub 29 to 35 unconnected); repeated pad numbers checked on layers
where they are copper of their own (`Board::merged_pads`); no relief cuts a
spoke in the DRC track-end test; cut-off spokes bring no item; hatched zones
(`Plane::hatch`: spoke end half a web out, a ring around thermal pads,
KiCad's hole grid); coincident pads share tracks; foreign spokes across a
pad's ring count (`PCB_ROUTER_NO_FOREIGN_SPOKES`); tracks and vias are items
of every piece their copper overlaps (`PCB_ROUTER_OLD_TRACK_PIECES`); 3 micron
tolerance on brush clearances; joined outlines for a pad's overlapping spokes
and lattice-cut necks (`PCB_ROUTER_NO_NECK_JOINS`). Router fixes: `reroute()`
refreshes the pours after negotiation; `trim_dangling_plane_ends` cuts a
plane tail whose fill was taken back (USB_Cable_Tester /SHLD: a layout move
rerouted signals over the plane end, not fill optimism).

Route-mode pairs against x738 (pinned, same time): Castor starved 4 to 3,
k30 1 to 0, mackerel-30 3 to 2, Hub 4 to 4 with 29 unconnected (x761 and
x765 had 35 from spoke frames in the guards, fixed in x767); CPU 0.94 to
1.05. Quick tier `build/quick30-x768.log` against x746: pass 22 to 23
(USB_Cable_Tester), starved 7 to 6, vias 2847 to 2814, unconnected 22 to 25
(reCamera 4 to 9, its ladder went another way; MIDAS 11 to 9). Eleven router
tests, one per class. `experiments/pour-fidelity/render.py` draws KiCad's
fill against our map around a pad.

Fragmentation measured on 7 boards: our extra pieces are lattice sampling of
necks; PixelWave's 1212 single-node blobs hold no spoke and no item; of 23
island spokes in outlines our map split, 4 pads had a wrong verdict and the
neck and spoke-overlap joins fixed 3. Remaining disagreements by cause:
0xCB U3.45 thermal vias inside the exposed pad (10 misses, modelled since in
"exposed-pad thermal vias"; modelling them
made 15 false alarms); seams between overlapping same-net zones of
different priority (Telemetry, 5; modelled since in "zone seams"); marginal fine-pitch clearance within a
few microns; hatched fill lying over thermal pads without relief (not
understood); per-track DRC subtleties with islands.

## Committed on 2026-10-14 (the determinism agent, x746 / x747)

Video's 262-392 unconnected for one placement was the layout's wall-clock
deadline (1.1 x 1500 s), not a race: in the four room-pair runs the
negotiation is identical to its last iteration and the runs part where the
deadline cut the hard repair (48, 70, 72, 98 nets rerouted). Video's work
lagged its wall clock (placement: 193 s of work, 630-1385 s of wall clock;
its seeds run in waves, each as long as its longest seed), so the deadline,
not the work, ended its layout under any load. Other wall-clock decisions
found: the via reduction's allowance in a layout (a third of the wall clock
left: Castor's final ladder 196 s on x738, 260 s of work now), the default
900 s negotiation of the attempts the ladder repeats from a probed rung
(wall clock only: video's narrow-signal step, 364 against 339 unconnected
on two runs), the pour nets' one-minute pre-negotiation in route mode, and
the probe ladder's check of the work left only between waves (the number
of rungs judged followed free memory). Route mode (Castor, Sisu: three runs
each) and the small boards' layouts were already deterministic.

Changed: the layout's routers stop at its end in work (router
`deadline_work`, armed before every phase; the end is `total_seconds`, the
wall-clock deadline 10 % later is the load margin); the waves of placement
seeds count toward that end (`waves_work_seconds`); the first route's
negotiation gets at most half of what is left; the via-reduction allowance,
the default negotiation budget and the pour pre-negotiation are in work;
probes are judged one after the other whatever ran side by side; every
wall-clock cut is logged (`wall guard: ...`) and reported (`cut` in the
reports and in run.py's rows, CUT in compare.py); `PCB_ROUTER_NO_WALL_GUARD`
drops the layout's deadline too. Video (seed 1, four threads): three runs
on eight logical CPUs 300/300/300 unconnected, 1362 vias, one fingerprint,
1355-1475 s; on four pinned CPUs it still meets the deadline (placement
alone 980-1385 s) and says so; without the guard there four runs of x744
and x746 give 318 and one fingerprint. Quick tier x746 (load 7-30): all 30
rows identical to x722, 22 pass, CPU geomean 0.97. The waves' count followed
the cores the process may use, so a run pinned to fewer cores could end
elsewhere than an unpinned one (fixed by the placement pool, below). Left: the clean-up's pour checks are not
work (a route-mode budget guard fired once under load 56, on a losing
rung). See docs/router.md, "Determinism and the wall clock".

## Triage of the v15 regressions (2026-10-14, no code changed)

A read-only triage of the 16 boards v15 (x723) lost against v14 (x510), by
pinned route-mode pairs on the same placement, layout repeats, and layout
with `solid_pin_rows` off. Outputs in `build/triage/` and `build/triage-*/`.

- The router is not the cause. On the same placement x510 and x723 give
  identical tracks-mode results (counts, vias, length; x723 reports fewer
  starved thermals), connect mode differs by a few connections either way.
  CPU x723/x510 0.98-1.06. Every x723 layout repeat reproduced its v15 row
  exactly unless the wall deadline fired.
- Ladder budget cliff (module 8 to 81 open, jetson 9 to 25, vaio_re): the
  last ladder step, signals at the project's narrowest predefined width plus
  the relaxed clearance, is what nearly completes these boards (module 126
  to 17 open, jetson 48 to 6 in route mode). In layout mode the first route
  eats the budget and the step's guards (`no_time_for_another`,
  `spent < 1.5 * ladder_budget`) refuse it. Assigned: reserve the step's
  cost or run it as an early rung.
- Placement perturbation (RP2040_base 100 to 173, Hub, SNSP, glasgow): a
  rule change shifts the legaliser's path and the race picks another seed;
  v14's RP2040 placement is legal under `solid_pin_rows`, the rule only
  perturbs (off: 100 open again). Seed-repeated pairs are the only sound
  judge of such changes.
- Wall clock not booked as work (0xCB panel, glasgow, vaio_re): the
  legalise phases book 0.10-0.15 of their wall time as work (panel 504-966 s
  wall for 34 s of work), so the layout deadline fires and the result
  depends on machine load (vaio_re 63 or 151 open for one placement).
  Assigned.
- Tabbed panels (0xCB panel, OpenESC 4in1, molecule panels): one outline
  piece, so `keep_on_pieces` never fires and 15-22 parts of one copy land
  on another copy; the router then reports 49 of 188 nets without a path.
  Assigned: confine each net group to its own fixed parts' region.
- Far-side courtyards (woagboard 26 courtyard overlaps; koeg-board and
  Quanta75 may share the cause): `legal.rs` `clear_of` checks far-side
  rects only between parts on opposite sides; KiCad checks two back-mounted
  parts' front courtyards against each other. Assigned.
- protoesp and vaio_re lost because v14's placements put parts inside
  module and connector bodies that `solid_pin_rows` now blocks (honest).
- None of v15's 45 passes relies on a bug: `inside_bodies.py` finds only
  fixed designer parts inside solid boxes, the designer-budgeted errors are
  internal to footprints. Caveat: qfwfq's hole_clearance sits at KiCad's
  199 cap, like the designer's.

## Committed on 2026-10-14 (the routing-room agent, x737 / x738): measured, built, off

Halo granted against halo used on 35 routed boards
(`experiments/routing-room/halo_use.py`, `analyse.py`; a part's band is
its body grown 1 mm; a 25 s probe's conflict and history maps from
`export-congestion-sample`): pin count does not predict where routing goes
(Spearman about 0 per board), RUDY in the band does (0.4-0.84, median 0.6)
and the probe's history likewise; 7-34 % of halo area is used and 0-74 %
idle (zpn 33 %, keyboar_ 48 %, 0xcb-static 74 %); keeping the half of the
halo area with the highest RUDY keeps 58-79 % of the routing inside halos
on crowded boards, the half by pin count about 50 %. `routing_room` (off;
`routing_room_share` 1.0, `_floor` 0.5, `_most` 2.0, `_all_parts` on;
`crates/pcb-placer/src/room.rs`): per-part demand from RUDY over each
net's spanning tree, halo = pin-count halo times min(2, k demand/median),
k by bisection so the total stays at share times the level's area; after
the normal placement a re-anneal with small moves, legalise, refine; a seed
whose pass fails keeps pin-count halos and sorts after the others. Seed
pairs (seeds 1, 101, 201, pinned): the option never fires on video,
OpenESC and hackrf (their kept level has no halos; hackrf walks down to
halo 0 because one part finds no room); on BAGEL, MK2.1, MK2, Castor and
zpn passes over 15 runs off 11, on 11 (BAGEL and MK2.1 win, where the level
had stripped small parts' halos; MK2, Castor and zpn lose 5-9 % HPWL as
busy clusters spread; zpn gains 1-3 mask bridges); CPU geomean 1.15x (the
second anneal on every seed). Share 0.5 gives HPWL -8 % on all five without
the routing collapse of the uniform x507 scale, but two passes lost to
single errors (within seed noise). Quick tier x722 to x737 with the option:
22 to 24 pass, both single-error flips (USB_Cable_Tester's track_dangling,
eternal-keypad's shorting_items), vias -3.7 %, 8 boards identical. Verdict:
implementation sound, idea as built a wash; left off. Found on the way:
video's routing is not deterministic for an identical placement (392, 314,
325, 262 unconnected for seed 1). Logs `build/room-*`,
`build/quick30-x737.log`.

## Committed on 2026-10-14 (the island agent, x722 / x723)

The islands are real in KiCad's fill, not our fragmentation
(`experiments/pour-fidelity/islands.py`): signal tracks and vias on the
plane layers cut them off in the tracks rung. KiCad experiments (kicad-cli
on mackerel-30 C50.2): a pad's own track ending inside the island cures
the finding, as does a track crossing it; a plated pad's fill stops at the
drill, so a track leaving a round through-hole pad between its diagonal
spokes touches none of them (the fill agent's "own track touches every
spoke piece" was wrong there). Changes, all on: `track_meets_spokes` (a
pad's own track joins a spoke's piece only where its first run overlaps the
spoke's strip outside the drill; census found 116 to 124, missed 36 to 28,
false alarms 87 to 90; `PCB_ROUTER_OLD_TRACKED`), `island_vias` (a short
pad whose manual spoke reaches no kept fill gets a stitching via from its
island to the net's main piece on another layer, also in tracks mode;
`PCB_ROUTER_NO_ISLAND_VIAS`), `island_stubs` (failing a via, a straight
track of the pad's own into its island, slivers included;
`PCB_ROUTER_NO_ISLAND_STUBS`). The stub changes no connection: the island
already hangs off the pad; it only satisfies KiCad's two-items rule. Its
broader form (stub every island-only pad) was rejected: 18 pads KiCad
passes for every 1 fixed. USB_Cable_Tester's bridges were not a pour
problem: KiCad exposes copper within `solder_mask_to_copper_clearance`
plus the aperture's expansion (0.1 mm there; our model had 0.075); now read
from the project (`solder_mask_to_copper_mm`) and used as the opening
clearance, always on. A pour keepout under openings exposing another net
was measured and not built: 1 real case in 128 results against 475 false
firings (`mask_bridges.py`). Pairs x710 to x722 (route mode, v14
placements): k30 starved 14 to 1 (+10 island vias), mackerel-30 19 to 3,
Castor 5 to 4, Hub 4 to 4, USB_Cable_Tester bridges 3 to 0; CPU within
±4 % paired. Quick tier x693 to x722: 21 to 22 pass (mackerel-08), 29 of
30 same, unconnected 22 to 22; USB_Cable_Tester's race picked another seed
whose /SHLD connect branch ends where KiCad leaves no fill (pour optimism,
one track_dangling). Remaining starved pads explained pad by pad in the
agent's report: spoke landing and piece fidelity (28 missed, 90 false
alarms), repeated pad numbers, a fine-pitch spoke-clearance case (k30
U14.12), slivers with no manual path. Reference tier log now
`build/quick30-x722.log`.

## Committed on 2026-10-14 (the legaliser agent, x701 / x711): the premise was wrong

The legaliser does not lose 30-90 % of the global placement's wirelength.
Measured with `PCB_PLACER_TRACE` (per seed the weighted HPWL after global
placement, annealing, legalisation and refinement, plus the designer's for
the same movable set) on the 30 tier boards and nine gap boards: the
global placement is 30-90 % longer than the final one and the annealer
removes that (annealed/global median 0.83); legalisation costs 4 % (median;
Castor 25 %, MIDAS-MK2 28 % before refinement, where the nearest-spot
legaliser moves 100-120 parts because larger parts placed first take
their spots or a fixed through-hole part's far side is met); refinement
wins most of it back (final/annealed median 1.00). Final/designer median
0.85; above the designer only BAGEL 1.77, zpn 1.39, MIDAS-MK2 1.34, MK2.1
1.28, hackrf 1.19, Castor 1.14. Video (0.96), OpenESC (0.61-0.73) and SNSP
(0.85-0.93) are below the designers: their failures are not wirelength.
The gap to the designers on crowded boards is the routing halos: no halos
gives -22 % HPWL, and designers' own placements are illegal at our halo
level for nearly every part (they put courtyards against each other). Four
legalise-or-place-better variants (reflow only conflicted parts, push apart
a step at a time, filler whitespace, four times the annealer's moves) do
nothing beyond seed noise (about ±10 % per board); removed, numbers in
docs/placer.md. Two options built, both off: `anneal_far_side_gaps` (the
annealer sees the legality test's hole and far-side gaps; Castor's invisible
conflicts 33 to 0, parts moved by legalisation 119 to 66; tier flat, video
and OpenESC worse, SNSP better) and `shared_halos` (neighbours keep the
larger halo, not the sum; six gap boards HPWL -6 %; tier 19 to 20 passes,
unconnected 29 to 24, vias -8.8 %, but zpn loses its pass and hackrf +57 %
vias). Placement with defaults is byte-identical to x652. Lesson: a
placement change flips crowded boards by ±75 connections at random, so
seed-repeated pairs are needed before any placement default changes. Next
lever: routing room from congestion (a halo only where RUDY or the first
route says a part's escapes need it) instead of the pin-count rule; less
halo everywhere has now lost routing twice (x507 halo scale, shared halos).
Scripts `experiments/placement-loss/`; logs `build/legal-*`,
`build/quick30-x700*.log`.

## Committed on 2026-10-14 (the fill agent, x693 / x710)

Starved thermals were not optimistic fill misses: in a census of 44 routed
boards (4274 thermal pad-layers, 171 DRC starved findings;
`experiments/pour-fidelity/{spokes,census,classify,causes}.py`, `spokes.py`
counts spokes in pcbnew exactly as the DRC does, `dump-pour-map` writes
`<net>.spokes`) the old model predicted 77 of the findings but raised 866
false alarms, so the router could not tell real starved pads from false
ones. `kicad_spokes` (on; `PCB_ROUTER_OLD_SPOKES` for A/B) copies KiCad
10.0.7's zone_filler and zone-connection DRC: spoke end at gap + half the
minimum width, spoke width min(spoke, pad minor), landing test off the
lattice with exact distances (`router/fill.rs` FillProbe), clearance from
the DRC ring, facing spokes of neighbouring pads meet, the island rule
("single" = exactly one item touches the piece), and the DRC's rule that a
track leaving the pad into kept fill counts as a spoke. New census: 116
predicted, 36 missed, 87 false alarms. `manual_spokes` (on;
`PCB_ROUTER_NO_MANUAL_SPOKES`): after stitching, a pad our model finds
short of two spokes gets straight tracks of its own class into a piece
KiCad keeps (the DRC counts them as spokes); also for pour nets routed as
tracks. Quick tier x651 to x693: 19 to 21 pass (Castor, spell_tome,
rp2040zero), starved 12 to 2, unconnected 29 to 22, vias 2859 to 2838, CPU
ratio geomean 0.87 under different load (no slowdown). USB_Cable_Tester
lost its pass: GND fill under U1's F.Mask polygon (two solder-mask
bridges), a routing change from the spoke model, not the repair; the pour
under a mask opening of another net is the gap the bodies agent noted.
Starved-only boards in route mode: Castor 20 to 5, PixelWave 30 to 1, Hub
15 to 4, rosco 9 to 3, k30 21 to 14, mackerel-30 21 to 19. The rest:
inner-layer through-hole pads whose spokes reach only island pieces (k30,
mackerel-30; needs stitching of single pieces in tracks mode), island-rule
edge cases, pads with repeated numbers and custom spoke templates (19 in
the census, unmodelled). Cell-level fill agrees to 1-3 % but our map is in
far more pieces than KiCad keeps (Castor F.Cu 128 against 17); untouched.
Logs `build/quick30-x693.log`, `build/fill-starved`, `build/fill-census`.

## Committed on 2026-10-13 evening (the bodies agent, x652)

Census of the 109 boards: 3800 footprints with pads and no courtyard
(2591 movable) on 92 boards; they blocked only with their pads, so hackrf's
LQFP-144 held 32 parts and homebrew_486's ISA slots interleaved. A Fab-layer
body rule fails (hackrf's library and the ISA16 draw none; where a Fab box
exists, 42 of ULK's switches hold a diode on purpose). The pad-pattern rule
`solid_pin_rows` (on): a footprint with 4 or more pads, three quarters of
them in rows or columns of 3 or more at most 2.6 mm apart, blocks with its
whole pad box plus copper and mask graphics; 625 footprints are solid, and
only 17 hold a designer's part (16 fixed-on-fixed). Keyboard switches,
hot-swap sockets and stabilisers stay hollow. `mask_only_pad_openings`
(router, on): a pad with a mask layer and no copper is a mask opening of
the net under it (hackrf's test points: its two solder-mask bridges on the
designer's placement become a pass). Copper-edge floor of 0.02 mm in the
placer (KiCad flags contact at zero clearance). Quick tier x651 against
x644: 19/30 both, unconnected 29 both, 21 boards byte-identical; OpenFC,
USB_Keypad and WLED change because parts no longer sit inside a QFN-80, a
USB-C receptacle or a LED pad box. hackrf: 0 open, one starved thermal
left (P5 VCC on the inner pour layer), but 1.6x the routed length now that
its parts are legal (crowded placement, problem 4); homebrew_486 passes
with its slots apart. Latent: the copper-pad aperture rule reads
`solder_mask_margin`, a key KiCad never writes (`pad_to_mask_clearance`);
noted in a comment. Logs `build/quick30-x644.log` (the reference),
`build/quick30-x651.log`, `build/bodies-*`; scripts `experiments/bodies/`.

## Planted boards (2026-10-13)

Boards routable by construction, three ways (`experiments/planted/`,
`docs/experiments/planted-boards.md`): maze (unrealistic), grow (realistic
at the easy level, cheap, but every one of 48 boards is easy for us: ours
shorter with fewer vias on 45), derived (blockers in a routed real board;
the only set that exposed failures, all starved thermals on complete
routes). Planted tile occupancy is a weak congestion label (Spearman
0.05-0.23 against our routing). No network; grow needs real difficulty
(pours on 2 layers, BGAs, buses) before its results are worth training on.

## Committed on 2026-10-13 (the placement-gap agent, x644)

Five boards that pass with the designer's placement and not with ours
(zpn_devboard, video, MokyaLora, hackrf-one, homebrew_486; logs
`build/gap-*`), each run pinned with ours against the designer's placement.
Wirelength is not the gap: our HPWL is within 0.8-1.4x of the designer's on
all five, and on four the loss is a modelling or task bug.

- **MokyaLora** passes now: a movable footprint's own rule areas (a U.FL's
  keepout) now move and turn with it, in the writer and in `run.py`'s
  stacking (`move_zones`). About 15 of the 109 tasks change through
  `run.py`, so baselines made before this differ on those boards (three
  tier boards remeasured: `build/quick30-x510-zonetasks.log`).
- **zpn_devboard** passes now (two runs, 0 open, 291-299 vias against the
  designer's 136): `keep_on_pieces`, on by default, keeps the parts a net
  joins to fixed parts on one piece on that piece; before, most of the
  board had moved onto an empty breakaway (129 open after the first route).
  Nothing changes on the tier.
- **hackrf-one** loses its 6-9 `items_not_allowed`: a movable part's keepout
  that forbids pads keeps the far side's pads out; a board rule area on
  inner layers only that forbids pads keeps pads out on both sides (a
  through-hole pad crosses them). It still fails on two solder-mask
  bridges from mask-only test points the router does not treat as
  obstacles (the designer's placement routed by us shows the same two).
- **homebrew_486** is not a gap (plus or minus one unconnected run to run);
  its v14 pass is flattered: a courtyard-less footprint blocks only with
  its pads, so the second ISA slot interleaved the first 2.54 mm apart,
  and on hackrf a header and an SOIC sat inside a turned LQFP's pad ring.
- **video** is the one real objective gap: eight SIMM sockets in one column
  at the designer's 8.89 mm pitch line the bus up; we scatter them.
  `bus_rows` (identical footprints sharing a bus pin for pin placed as a
  row; through-hole only; off) gets video from 315 to 259 open at the
  designer's pitch and 499 with the automatic one, made mackerel-08 worse
  (+6 starved, +21 vias, +1100 CPU s) and homebrew_486's SRAM rows lost two
  connections; left off. Video needs more than aligned sockets.
- Rows whose members block with shaped boxes now block with every member's
  boxes (before only the first part's; likely the J9/U16 overlap on 486).

Quick tier x642 against x510: 19/30 both, unconnected 30 to 29, other
copper errors 8 to 3, starved 12 to 18 and +21 vias all from the bus rows
on mackerel-08 (now off); 25 boards byte-identical. Next on this thread:
courtyard-less ICs and connectors should block the inside of their pad
box (not a blanket rule: ULK puts 42 diodes inside its switches' pad
boxes).

## Committed on 2026-10-10 (the memory and congestion agents)

- Rungs side by side with fixed work budgets, honest via-reduction budgets,
  a 4-ary key heap (9-13 % less search CPU), polish-again after a trial's
  polish, and the layout deciding by work everywhere: a board's layout now
  reproduces exactly run to run (CyberKeeb 3 of 3, Sisu byte-identical).
- The placement race pre-ranks 16 placer seeds by RUDY (routing demand over
  tile capacity) and probes the best three: quick tier KiCad unconnected
  98 to 64 (6 boards better, 2 worse), laptop power finds a legal seed.
  Default from the next commit. The neural predictor ranks race candidates
  about as well as RUDY (its label, a single 75 s probe, agrees with
  itself only at Spearman 0.6 between router seeds) and stays an
  experiment behind `congestion_model`; see docs/congestion-model.md.
- Placer correctness (the placement agent, in 3b890fa and 93b83f5): a
  fixed part's copper and mask graphics on its other side are obstacles
  there (Castor's logo: the short is gone); parts at odd angles are tested
  as turned rectangles and may turn square (ErgoSNM places: 58/58 clean);
  a footprint with a courtyard on each side gets a body per side (urchin
  places: 66/68); courtyards may touch where the pads inside keep the
  copper clearance, as a last resort (laptop power places on every seed,
  but RUDY then prefers a touching seed: 9 to 18 unconnected, a ranking
  fix is under way); rule areas inside footprints and pads-only rule
  areas are lowered (PixelWave 41 keepout findings to 0, 343/343 routed;
  d20 27 to 0); eviction 7x faster. framework_mobo's M1 has no legal spot
  at the designer's outline (its pads overhang the polygon): the task
  generator should test overhang against the polygon. SmartSpin2k: 16
  seeds cost 1218 s of placement on 4 cores (a placement budget for the
  seed count is under way). Measured: the legaliser's wirelength loss is
  mostly won back by refinement (final within 10 % of the global
  placement on most tier boards); the 1.4-1.9x against the designers is
  the routing halos, whether less halo still routes is the next experiment.
- Placer round 2 (a609a75): a touching-courtyard placement ranks last;
  the race's seed count follows the placer's 40 % share of the work (the
  placer's work clock was undercounting legalisation 20-fold; fixed);
  courtyards that fill under 75 % of their box block with their shape
  (ghoul places, urchin passes). Quick tier x617 against x500 (everything
  since x500 together): passes 16 to 20, KiCad unconnected 67 to 29,
  placement failures 4 to 1 (framework M1, the board's). The task
  generator now tests overhang against the outline polygon: 35 of 109
  tasks gain fixed parts (mounting holes in rounded corners, parts in
  notches); tasks v6 for v14. Open: `halo_scale` experiment (does less
  routing halo still route), the placer's model being stricter than
  KiCad at mounting-hole rings and pad boxes (45 designer poses illegal on
  ghoul), SmartSpin2k (a 585-connection panel whose first route alone
  needs 1000 s; not a target).
- Halo scale measured on the 30-board tier (x507): 0.75 and 0.5 cut the
  placer's wirelength only 1.3 and 4 % (halos are already shrunk where
  boards are crowded) and win nothing clearly (passes 19 / 19 / 17);
  1.0 stays. The 1.4-1.9x gap to the designers' wirelength is not the
  halos. The race's judge: the 75 s probe is the best pre-routing
  predictor of a layout's final result (Spearman 0.55 on 92 finals; two
  probes averaged no better; RUDY 0.30, wirelength 0.23, the network
  0.13), so the network is a closed negative as a ranker and
  `probe_seeds: 2` is not built.
- Speed is structural now: cheaper expansions are exhausted (five
  candidates under 5 %). The waste measurement (`PCB_ROUTER_WASTE=1`,
  bench-search) found chronically rerouted nets searching corridors up to
  10 tiles wide for narrow paths, and on Sisu and jetson half the
  expansions after iteration 20 gaining nothing. Committed (3c1f81d): the
  corridor margin grows every sixth reroute instead of every third: quick
  tier 10 % less CPU, unconnected 32 to 30, starved 20 to 15, vias -6 %,
  no pass lost; SNSP route mode 171 to 98 unconnected at +6 % vias. A
  stall stop (N=12 iterations without a conflict improvement; 3-11 % less
  CPU, boards nearly unchanged) is queued for a tier pair. Capping the
  margin fails (SNSP needs the wide corridors).
- Committed aa4e9e8: the negotiation ends twelve iterations after the
  price cap without a conflict improvement (tier -8 % CPU, -18 % board
  time, no pass lost). The 30-board tier's clean baseline on x508: 19 of
  30 pass, 23 unconnected, 12 starved thermals, 31.5 min of wall at 8
  jobs (`build/quick30-x508.log`).
- Speed, round three (2026-10-13; experiments in the worktree
  `pcb-maker-speed2`, diff `build/speed2/speed2-experiments.diff`, logs
  `build/speed2`, tiers `build/quick30-x333-L12.2.2.log`,
  `build/quick30-x334-viastop.log` against `build/quick30-x510.log`), all
  negative:
  - Chronic nets searched near their last route (`PCB_ROUTER_LOCAL=12,2,2`:
    a net rerouted 12 times or more searches its corridor cut to two tiles
    around its previous route, tree and targets first, every other
    reroute). Measured first: nets with 18+ reroutes find their path within
    one tile of the previous route in 97-98 % of searches yet expand 72-78 %
    of their nodes outside it. bench-search reaches lower conflict counts at
    equal work (Sisu 17 at 350M expansions against 28; jetson 9 at 550M
    against 25) and stops at 58 % / 82 % of the expansions. But the tier is
    neutral (19/30 both, unconnected 30 to 24, CPU -5.5 % against x510 and
    +9 % against x508: noise), Sisu in route mode -10 % CPU (pinned pair,
    swapped: 0.894, 0.915) ends 34 open against 27, jetson identical (2
    open) at -3 %. Route mode rarely reaches the chronic regime; where it
    does, the full searches are what finds the detours.
  - Not re-searching a conflict-free net: already so (a net's branches are
    tested for conflicts at its turn, after the nets before it moved; a
    speculative search that read a changed block is redone).
  - A cheaper first iteration: iteration 0 is 1-2 % of the negotiation's
    expansions on Sisu and jetson, the first three 3-7 %; not built.
  - On the tier the negotiation is not the cost: the via reduction is (its
    lines sum to 66 % of the v14 tier layouts' routing time, the clean-up's
    to 27 %; 80 % of its rounds reverted, 72 %
    because the round found no fewer vias, and after such a round the next
    is kept 9-11 % of the time). Stopping at such a round
    (`PCB_ROUTER_VIA_STOP`) cut the tier's via-reduction time 13 % and its
    CPU 1.7 % (noise), +18 vias: a layout spends its work budget anyway
    (more move trials), so a cheaper phase shows up as more trials, not as
    less CPU. Speed on the tier means quality per work, and a phase that
    ends by its budget gains nothing from being cheaper.
- The quick tier is 30 boards now (`benchmarks/agent-tasks/quick.json`,
  README): real layouts in about 1100 s and 3.7 GB; about 35 min at 8
  jobs.

## Committed on 2026-10-09 evening (b67da0a), measured by the pour agent

- Spoke clearance (a spoke counts only when its strip clears other
  nets' copper), pour-aware clean-up and via reduction (a change that
  strands a pour pad is rejected), stale plane stubs rerouted,
  deterministic stitching, the pour-refresh panic fixed at its root, circle
  rule areas lowered. k30 forced to connect: 41 open (before) to 0 open
  with 14 starved thermals, beating the tracks rung's 16. Castor's connect
  rung in route mode: 126/126 with 18 starved; Hub 144/159 with 20.
- Measured and left off: stitching vias during negotiation (Sisu 63 to 68
  open: the stitches crowd the signals), main-piece hysteresis (the main
  piece does not flip, it collapses when a stuck pour net is ripped up
  whole), not ripping pour nets up whole (worse), hard thermal guards.
- Cost: the pour-aware check's own-stamp lookup is a bitset now (k30
  forced-connect checks 179 s to 78 s, Hub 207 s to 130 s, identical
  results); skipping far pour nets never fires on boards whose pours cover
  the board.
- Same-copper comparison, route mode on v12's placed boards, x165 against
  HEAD: k30 identical (tracks wins both), Castor 184 to 176 vias at 18
  starved, Hub 18 to 20 unconnected, Sisu 20 to 13 unconnected. The
  probe ladder picks the tracks rung by its 75 s probe even where the
  connected rung's final result is better (k30 forced connect: 0 open,
  14 starved against tracks' 16): the ladder's judge has the race's
  noisy-probe problem.
- Real-layout rows on x204 (fixed run.py): k30 173/173 with 3 starved
  (365 vias to 287), Qfwfq 64/64 clean, Telemetry 107/108, Castor 125/126
  with one unconnected, one short and one bridge (the short is under
  investigation), Hub 132/159 with 40 (placed for the first time), Sisu
  183/192 with 12 (unchanged: it was always placed).

## Sweeps before v16 (2026-10-09 to 2026-10-14)

**Layout sweep v11** (`benchmarks/agent-tasks/run.py` on
`build/github-tasks-v5/tasks.json`, binary x136 with killed or panicked
boards rerun on x137-x146; 109 boards harvested from GitHub, each reduced to
stacked parts plus a `constraints.json` derived from the designer's board,
1800 s each): **38 of 109 pass** (KiCad: 0 unconnected, no copper error,
constraints met). Against the x112 baseline on the 29 boards both have:
better on 14, same on 13, worse on 2 (the two OpenESC/OpenRX panels), 1598
to 1231 unconnected, passes 5 to 5. Reproduce the table with

    benchmarks/agent-tasks/compare.py build/github-layout-v8-x112.log build/github-layout-v11.log \
      --splice build/github-layout-v11-x138.log build/github-layout-v11-x139.log \
               build/github-layout-v11-x140.log build/github-layout-v11-x141.log \
               build/github-layout-v11-x146-jiran.log --markdown

The full table is in `docs/benchmarks.md`.

**A caveat on every layout sweep up to v12 (found 2026-10-09 by the
congestion-model agent):** `run.py`'s unplace only read KiCad 10 net
syntax, so on the 89 pre-KiCad-10 tasks every footprint counted as netless
and stayed where the designer put it; only 22 of the 109 tasks were really
placed (the v12 logs' "placed in" times show which). Fixed in run.py;
v13 is the first sweep that places all of them, and its numbers will be
lower. Route-mode conclusions (pour model, connect rungs, Freerouting
comparison) are unaffected.

**Layout sweep v15** (x723, tasks v6, 6 jobs, 17910 s, 2026-10-14): **45 of
109 pass**, 3829 open; against v14 better 20 / same 63 / worse 16; the
worse rows are the crowded boards (see docs/benchmarks.md).
**Layout sweep v14** (x510, tasks v6, 10 jobs, 12668 s): **34 of 109
pass**, 3065 open in all; against v13 better 19 / same 51 / worse 24. The
baseline for what comes next (`build/github-layout-v14.log`; tasks
`build/github-tasks-v6`). Timeouts: link, katia, MegaDrive, the 0xCB
panel; no legal placement: the molecule panels and the OpenESC 4in1 panel.

**Layout sweep v13** (x503, 12 jobs, load 25-40, 14525 s): **31 of 109
pass** with every board placed from stacked parts; 3586 open in all. The
honest layout baseline from now on (v12's 41 was mostly routing). 7
placement-illegal rows (5 molecule panels, framework M1, ghoul), 6
timeouts (katia, PixelWave, SmartSpin2k, MegaDrive, two 0xCB panels).
v14 should run on the 16-seed race (x504+) once the placement budget for
the seed count is in. Table in `docs/benchmarks.md`.

**The first real-layout baseline, quick tier** (`build/quick-v13-base.log`,
x165 with the fixed run.py, 37 boards, 12 jobs, 3617 s): **18 of 37 pass**
(25 when the designer's placement was kept). New failure classes now
visible: four boards fail before routing because the placer finds no legal
spot for a part (framework_mobo_lefthalf M1; laptop power C8/R49/C22;
ErgoSNM KEY31/KEY33; urchin SW31/SW23); the SmartSpin2k panel at 270/585;
WLED_Matrix 26 and PixelWave 30 unconnected with shorts, bridges and 50
keepout violations; MIDAS-MK1 15, BAGEL 7. Starved thermals only:
mackerel-08 8, Castor 9, 1337-v4.0 3, bumwings R64 1. The tier's stored
times and peaks (quick.json) are from designer-placement runs and must be
redone (one board peaked at 5.7 GB against 3.7 assumed).

**Layout sweep v12** (x165 = 1c7a557, 14 jobs, 10375 s in all): **41 of
109 pass**; against v11 better 12 / same 77 / worse 12, unconnected 4722 to
4771 (SNSP +90 and Quanta75 +25 are the known costs of the mask-body and
plate rules; A13 +42: x141, x146 and x155 all give 162/223 with 103 under the
sweep's load and the same race as v11, x165 picks another seed (279
against 288 unfinished after the probe) and ends worse again; the board
is sensitive to the race's pick and to load, not to one code step). Seven
error rows: Telemetry's degenerate fill-keepout rule area (new lowering
bug), the `refresh_pour` -> `find` stale-index panic on SUMEC, coco2,
bumwings xiao_sd and Qfwfq, PolyKybd right at the 1800 s limit under
load, the 0xCB panel's kicad-cli timeout. Table in `docs/benchmarks.md`.
Compare: `compare.py build/github-layout-v11-spliced.log build/github-layout-v12.log`.

**Freerouting against our cold route** (`benchmarks/github/freerouting.py`,
headless, 1500 s each, both on the designer's placement with all copper
removed; `freerouting_compare.py`): all 109 boards. Freerouting finishes 70,
times out on 34 and fails on 5; where it finishes we have fewer unconnected
on 49 boards, it on 6; unconnected in all 913 against 206; clean boards 8
against 50. Our rows mix x136 (40 boards) and x155 (69); rerun the 40 on a
current binary when the comparison is quoted as final. Logs:
`build/github-freerouting{,-2,-3}.log`, `build/github-ours-cold{,-2,-3}.log`
(`-all.log` is their concatenation).

**Route sweep v16** (`benchmarks/corpus/run.py --skip-layout`, x136, 2700 s)
has 2 rows (jetson 338/340 with 2 unconnected, link 246/251 with 8) and was
stopped; restart with `build/chain-v11.sh`'s last line.

Rerun lists kept: `build/github-layout-v11*.log`, `build/github-freerouting*.log`,
`build/github-ours-cold.log`; binaries `build/bin/pcb-maker-x112` ... `x155`.

## What was learned this week, condensed

- **A knockout text is its plate** (the glyphs' box grown by the stroke),
  not its glyphs; a copper text with a twin on its mask layer is
  exposed-copper artwork and stays put, for the mover and for the layout's
  in-place route alike. Quanta75: shorts 0, the bridges left are the
  designer's J8 inside the logo. Routing less (146/172 against 169) is the
  price of the plate as an obstacle.
- **A footprint's own mask graphics are part of its body** for the placer
  (SNSP-CPU-01's J2: a 22 x 32 mm opening, 63 pads of other parts under it,
  each a bridge). The cost is real on that crowded board (333/363 with 116
  unconnected to 309/363 with 290; the race ends at 418 unfinished nets
  instead of 314), measured by bisecting (x143 reproduces x136 exactly).
  Kept: pads under a mask opening are bridges, and the back side stays free.
- **The ladder's memory** (x146): a finer pitch grows as the square of the
  pitch ratio and the seeds run side by side; jiran-ble-lite (3299 x 1048
  nodes at 0.1 mm, 13.8M at 0.05 mm, 7 GB a router) took 31 GB. The ladder
  now projects memory from the process's peak after the first attempt
  (`memory_mb`, `router_mb`): a finer pitch must fit 80 % of what is
  available, seeds half of it. jiran: 104/105, KiCad 20 unconnected (its GND
  has no pour; half its 40 GND pads are not reached on 2 layers; open).
- **KiCad's island rule** (x147-x155): a fill piece joined to the net by
  thermal spokes alone is an island; KiCad removes it and does not count the
  spoke ("spokes connected to isolated island"). Measured in pcbnew on
  k30-SBC: a via inside the piece cures it. Changes, all default:
  - `analyze_pours_full`: a piece gets copper of the net first (a track or
    via node, a solid pad); a thermal link joins a pad to a piece with such
    an item, never the piece to the pad;
  - every item-less piece a spoke reaches, for a pad short of two good
    spokes on that layer, gets a stitching via when one fits;
  - the net's own tracks are fill wherever fill may lie, inside a pad's
    thermal ring too (a plane stub ending in the ring read as "stranded, 1
    branch, 0 pieces" and was rerouted and stitched for nothing);
  - `canonicalize_copper_net_names` falls back to the bare name: copper is
    written as `(net "/GND")`, and since the net-name registry (x140) kept
    the slash when a bare GND exists, KiCad made a net "/GND" of our items
    (Hub: 636 items, 36 clearance errors);
  - `junction_hosts` skips a one-node branch between two junctions (the Hub
    panicked).
  Results: Castor 126/126 clean, islands 4 to 2 (x155; one run gave 5
  starved thermals in all, another 18: the winning rung decides); Sisu
  186/192 with 13 (x136: 185 with 26); Hub 146/159 with 17 (as x136); k30
  173/173 with 10 (as x136, its 8 one-spoke pads untouched).
- **Hard thermal guards do not work and cost routes.** Closing a pad's last
  two spoke corridors to other nets (`hard_thermal_guards`, off) left k30's
  one-spoke pads as they were and cost Castor six nets and Sisu fifty
  connections.
- **The finding under all of it: our fill is 10-40x more fragmented than
  KiCad's.** k30's GND fill is 63 + 33 + 29 outlines in KiCad (F, B, In2)
  and 1100-1500 pieces in our pour map; +5V on In1 24 against 338. The
  brush-centre model on a 0.1 mm lattice loses every neck near the minimum
  width (0.254 mm is 2.5 cells), and clearance rings around other nets' vias
  on a plane layer meet where KiCad's do not. The two-way pad links of the
  old model hid this; the one-way links show it as stranded pads, reroutes
  and stitching vias. k30's +5V: "38 stranded terminals" while KiCad finds
  every +5V pad connected.

# The move descent (2026-10-23)

Florian's idea: "if we are very fast with relayouting, can't we just
smoothly move individual components to see whether that helps (ideally
with some idea which ones would be good candidates)." The coupled loop
(docs/coupling.md) already moves parts after the first route, but a
trial there is a full incremental reroute (`Router::update` plus
`Router::reroute`), and on four-layer boards with pours more than 5 % open
it does not run at all (the fence benchmark pass: on DSP-ADAU1452 every
such trial left 224 to 285 open against 100). This experiment makes a
trial cheap and lets a descent of single moves run on exactly those
boards. Branch `move-descent`; snapshots x1690-x1692 (x1691 on main a49136c
for the dense and the first short-tier measurement, x1692 on main
5ef2fbc, the beaten-boards merge, for the short tier again).

**Verdict: no-go as a default.** The trials are cheap now (0.6-4.5 s of
work a router trial against 56-327 s for the coupled loop's) and the
descent lowers the router's own open count on every dense board where it
runs (DSP 93 to 66, 4in1 154 to 122, protoesp 126 to 103), but the final
boards do not follow: over the eleven dense boards KiCad's unconnected
items rise from 978 to 1026 (better 1, worse 5, the same 5), the losses
from the work the descent takes from the last route (DSP-ADAU1452 15 to
35: its continuation in the other pour mode no longer runs) and from a
descent board KiCad finds worse than the router does (SNSP-CPU-01 83 to
108). The short tier is unchanged in KiCad's counts. The switch stays,
off.

## What a trial cost (step 1)

Measured inside the layouts with `PCB_LAYOUT_DESCENT_CLASSIC=3` (the
descent's first three router trials also tried on a copy the coupled
loop's way: a clone, `Router::update`, `reroute` without the polish;
dev binaries on main 9a2a31e, single layouts at the task budgets, 2
threads, load 17-33):

| Board | lowering | clone | update (wall / modelled build work) | reroute: wall / search work | open after (before) |
| --- | ---: | ---: | ---: | ---: | --- |
| OpenESC 4in1-mini | 0.02 s | 0.01 s | 0.3-0.5 s / 0.18 s | 44-58 s / 56-61 s | 144 (123), 119 (115), 115 (110) |
| DSP-ADAU1452 | 0.02 s | 0.04-0.11 s | 1.3-2.1 s / 1.15 s | 172-440 s / 125-327 s | 70, 64, 85 (74) |

A trial of today's loop is a minute of work on the small dense board and
two to five minutes on DSP: the reroute negotiates the ripped nets at the
reached price and the hard repair after it re-settles everything they
pushed. Two of the three DSP trials did close terminals (74 to 70 and
64), so the moves are not useless; at 125-327 s each, the 600 s moves
budget holds two to four of them. video's first route ends at 1351 s of
its 1500 s of work: no moves budget is left there at all, by either
loop.

## The two-tier trial (steps 2 and 3)

`crates/pcb-kicad/src/move_descent.rs`, `move_descent` in layout.json
(absent: off; `PCB_LAYOUT_MOVE_DESCENT` overrides, `0` off).

- **Candidates** (re-ranked after every kept move): the parts with
  unconnected terminals of their own (the router's per-terminal state,
  `Router::unconnected`), then those whose nets have open terminals,
  then the parts with the largest overflow attribution in the global
  route (`board_placer::attribution` at 0.5 of the capacity, phase 2's
  measure). The trouble repair's victims are not recorded by the router;
  they are open nets after it and come in through the first two keys.
- **Moves** per part: nudges of 0.5, 1, 2 and 3 mm in eight directions,
  quarter turns in place (the part's angle options), swaps with a movable
  part of the same size within 10 mm; followers move with their leader
  (`sync_followers`); legal by `placer::legal::is_legal` at the
  placement's relaxation and not worse for near and relative constraints
  (as the coupled loop's moves).
- **Tier one: the incremental global route** (`pcb_global::Incremental`,
  new). The full global route of the board is computed once (0.02-0.42 s
  of wall; DSP 0.16 s, video 0.42 s). A move is applied to the lowered
  board in memory (the part's obstacles and terminal anchors carried by
  the rigid motion from the old pose to the new), the capacities of the
  tiles its copper touched before and after are measured again
  (`Capacity::rebuild_region`, equal to a full build on those tiles; unit
  test), and only the part's nets are routed again on the tiles against
  the others' usage at the negotiation's final history and price. Score:
  100 per predicted open net (crossing an overflowing boundary or with an
  unreachable terminal), 10 per track of overflow, plus the soft overflow
  above half the capacity, against the current placement's. 1-3 ms a
  move (4in1-mini 1605 moves in 3.5 s, DSP 709 in 1.5 s), counted as
  placement work from the global route's own counts as phase 2 does.
- **Tier two: a local reroute.** The best `confirm` (4) moves that tier
  one does not find worse are written into the board, lowered for real,
  and tried on a clone of the router: `Router::update_nets` (new) rips up
  only what the move invalidated and keeps of a net whose pads moved the
  legal branches not ending on a moved pad (a supply's other pads stay
  joined) and the state of nets without copper whose pads stayed (a pour
  net joined through its pour); `Router::local_reroute` (new) routes
  those nets again one by one with every other net's copper as a wall
  (the hard repair's search, no negotiation: nothing else moves, nothing
  conflicts) and the pour nets whose pads the new copper cut off. The
  board's open terminals decide: a move that leaves fewer is kept with
  the router's state. Work: the modelled rebuild of the maps plus the
  search, at most `local_seconds` (2 s) of search.
- **Calibration.** Every descent first runs a trial of no move; it must
  leave the open count as it was (`null_open` in the report). Two bugs
  showed there during development (pour-only nets reset by the update:
  +16 to +32 on every trial; nets that were open with no copper retried
  in every trial, eating the search budget).
- **The descent** (step 4): ranked candidates in turn while its work
  stays within `trial_share` (0.5) of the moves' budget (`move_seconds`
  capped at 65 % of the layout's budget less the work so far), stopping
  after `patience` (60) parts in a row without a kept move; after a kept
  move the parts near it (5 mm) and those sharing a net of up to eight
  pads with it may be tried again. It runs wherever the first route left
  terminals open, before the coupled loop: on boards far from complete it
  replaces the 5 % skip (the loop itself still does not run there), and
  the narrow-signal attempt or the final ladder routes the placement it
  leaves from scratch, as it routed the first placement. When a move was
  kept the layout's routes are the descent's (`Router::finish_as_is`: the
  pours stitched, no repair).
- **Report** (step 5): `move_descent` in board-layout.json (parts tried,
  trials per tier, kept moves with their parts and the open count before
  and after, work and wall per tier, why it stopped) and the verdict's
  `moves` (trials, router trials, kept, parts moved, open before and
  after).

Trial costs in the development runs (tier two, mean per trial):

| Board | tier-two work | tier-two wall | update wall | first route open -> after the descent | kept / router trials |
| --- | ---: | ---: | ---: | --- | --- |
| OpenESC 4in1-mini | 0.9 s | 1.0 s | 0.3-0.9 s | 56 -> 46 | 9 / 188 |
| OpenESC 4in1 | 0.9 s | 1.1 s | | 147 -> 121 | 9 / 83 |
| DSP-ADAU1452 | 1.9 s | 2.7 s | 1.3 s | 93 -> 66 | 11 / 154 |
| protoesp | 0.7 s | 1.4 s | | 126 -> 96 | 7 / 256 |

against 56-327 s of work for one trial of the coupled loop: a factor of
60 to 170. Where the work goes on DSP: the update's map rebuild (1.12 s
modelled, the static maps of every class built again in full) is more
than half; a rebuild of only the changed tiles is the next saving.

How good tier one is as a filter: over 771 router trials of the
development runs, the moves tier one predicted better were kept 21 times
in 518 (4 %), the moves it predicted equal 15 times in 253 (6 %). The
global route does not resolve a millimetre's nudge (phase 1's finding:
it cannot rank seeds of one placer); as a pre-filter it costs nothing
and drops the moves it predicts worse, but the ranking among the rest is
noise. The router trial decides.

## Measurement

Single layouts at the task budgets (tasks v7, 1500 s of work, pinned
rates 3.15e6 and 150e6), both sides from **x1691 on main a49136c**,
5 x 2 for the dense boards and 3 x 2 for the short tier, `--min-free-gb
10`, no lock, beside four other agents (load 17-35); the on side
`--layout '{"move_descent": {}}'`. Output: `build/desc1-dense-{off,on}`,
`build/desc1-short-{off,on}` (and `build/desc1-short2-*` on x1692). HPWL
is the half-perimeter over the nets of the final board (unweighted).

**Off reproduces main.** The placer's output (board-placer.json, timing
fields aside) is identical off and on on all eleven dense boards and all
twelve short-tier boards (the descent runs after placement), and off
equals x1652's (`build/labels1-short-after-x1652`) on all twelve
short-tier boards. With the switch off no code of the descent runs:
`Router::update` is unchanged (the partial keep is `update_nets`'s
alone), `finish` skips the repair only for `finish_as_is`, and the
capacity refactor of `pcb-global` gives the same capacities (unit tests;
the global route is used by the default layout only through the descent).
The fixture goldens do not move. Fingerprints are route mode, which
never runs the descent: unaffected.

### The eleven dense boards

| Board | KiCad unconnected off / on | first route open | strict pass | HPWL mm off / on | vias off / on | work s off / on | moves scored / router trials / kept, router's open before -> after |
| --- | ---: | ---: | --- | ---: | ---: | ---: | --- |
| OpenESC 4in1 | **79 / 75** | 154 | no / no | 1237 / 1282 | 159 / 175 | 1400 / 1434 | 3775 / 96 / 11, 154 -> 122 |
| OpenESC 4in1-mini | **35 / 38** | 61 | no / no | 740 / 776 | 144 / 144 | 1239 / 1307 | 2099 / 61 / 3, 61 -> 51 |
| split72_left | 38 / 38 | 38 | no / no | 12730 / 12730 | 1114 / 1114 | 1292 / 1326 | 96 / 10 / 0, 38 -> 38 |
| video | 175 / 175 | 242 | no / no | 32694 / 32694 | 1514 / 1514 | 1482 / 1482 | did not run (no moves budget) |
| DSP-ADAU1452 | **15 / 35** | 93 | no / no | 4049 / 4057 | 270 / 265 | 1297 / 2166 | 709 / 154 / 11, 93 -> 66 |
| Glasgow | **148 / 151** | 164 | no / no | 3094 / 3096 | 279 / 282 | 1496 / 1502 | 2051 / 52 / 2, 164 -> 160 |
| SmartSpin2k | 80 / 80 | 82 | no / no | 18861 / 18861 | 1793 / 1793 | 1388 / 1391 | 12 / 0 / 0 (2 s budget) |
| 0xCB panel | **53 / 54** | 82 | no / no | 5601 / 5607 | 244 / 255 | 1227 / 1107 | 142 / 31 / 1, 82 -> 76 |
| SNSP-CPU-01 | **83 / 108** | 123 | no / no | 19119 / 19119 | 1844 / 1739 | 1292 / 1335 | 424 / 69 / 2, 123 -> 120 |
| MegaDrive | 193 / 193 | 217 | no / no | 28995 / 28995 | 2135 / 2135 | 1583 / 1322 | 0 / 0 / 0 (no budget) |
| protoesp | 79 / 79 | 126 | no / no | 5676 / 5682 | 382 / 379 | 1424 / 1418 | 934 / 219 / 4, 126 -> 103 |
| sum | **978 / 1026** | | 0 / 0 | 132796 / 132899 | 9878 / 9795 | 15118 / 15788 | |

(SmartSpin2k places now on this base: 80 unconnected on both sides.)

Per board, what the descent cost (on side; the global route once, tier
one per move, tier two per router trial):

| Board | budget s | spent s | stopped | global route | tier one a move | tier two a trial: work / wall |
| --- | ---: | ---: | --- | ---: | ---: | --- |
| 4in1 | 87 | 87 | budget | 0.02 s | 2 ms | 0.83 / 1.19 s |
| 4in1-mini | 153 | 44 | patience | 0.02 s | 3 ms | 0.65 / 1.06 s |
| split72_left | 199 | 35 | patience | 0.12 s | 8 ms | 3.4 / 4.9 s |
| DSP-ADAU1452 | 289 | 291 | budget | 0.26 s | 2 ms | 1.88 / 2.25 s |
| Glasgow | 84 | 64 | patience | 0.22 s | 4 ms | 0.99 / 1.05 s |
| 0xCB panel | 300 | 302 | budget | 1.02 s | 0.94 s | 4.48 / 4.13 s |
| SNSP-CPU-01 | 230 | 231 | budget | 0.34 s | 6 ms | 3.28 / 4.69 s |
| protoesp | 209 | 125 | patience | 0.17 s | 1 ms | 0.57 / 0.74 s |

The 0xCB panel (466 footprints) is the exception in tier one: a move
there costs a second, the clone of the global route's state and the
capacity measure around a part on a panel this size; tier two is under
five seconds of work everywhere. Every calibration trial (no move) left
the open count exactly as it was.

Why the router's gains do not reach the result:

- **DSP-ADAU1452, 15 to 35.** Off, the narrow-signal attempt had 1033 s
  of work, left 73 open, and its continuation in the other pour mode
  (485 s) ended at 30 open: KiCad 15. On, the descent took 290 s (93 to
  66 open in the router's count), the narrow-signal attempt started with
  742 s, ran 729 s past it (a pre-existing overrun of that step, larger on
  this placement), and no continuation ran. The board kept is the
  narrow attempt's from scratch, not the descent's routes.
- **SNSP-CPU-01, 83 to 108.** No final step runs on either side (the
  first route ends at 1288 s of work). The descent kept two moves (123 to
  120 terminals by the router's count), and KiCad finds 108 unconnected
  items on that board against 83 on the first route's: the open signal
  nets differ (off D2, PA1, D7; on RESOUT1, D5, D6, PA1), i.e. the hard
  local reroutes leave nets apart in more pieces than the router's
  terminal count says. Not explained further.
- **4in1, 79 to 75** is the one gain: 11 kept moves, 154 to 122 open in
  the router, and the final ladder (609 s) on the new placement ends
  better. 4in1-mini (3 kept) loses 3, Glasgow (2 kept) 3, the panel 1.
- **No run:** video and MegaDrive have no moves budget left after the
  first route (the moves stop at 65 % of the budget); SmartSpin2k had 2 s.

### The short tier

On x1692 (main 5ef2fbc, after the rebase onto the beaten-boards merge),
off and on from that one binary (`build/desc1-short2-{off,on}`); the
x1691 run (`build/desc1-short-*`) gave the same rows on every board but
WLED_Matrix, whose placement the merge changed (its copper graphics in
the body):

| Board | KiCad unconnected off / on | first route open | strict pass | HPWL mm off / on | vias off / on | work s off / on | moves scored / router trials / kept, router's open before -> after |
| --- | ---: | ---: | --- | ---: | ---: | ---: | --- |
| 0xCB 1337-v4.0 | 0 / 0 | 0 | yes / yes | 1293 / 1293 | 73 / 73 | 101 / 101 | complete: no descent |
| USB_Cable_Tester | 0 / 0 | 0 | yes / yes | 1711 / 1711 | 106 / 106 | 134 / 134 | complete |
| USB_Keypad | 2 / 2 | 2 | no / no | 1625 / 1625 | 181 / 181 | 472 / 450 | 456 / 164 / 0, 2 -> 2 |
| WLED_Matrix | 8 / 8 | 12 | no / no | 1844 / 1844 | **194 / 271** | 407 / 451 | 646 / 162 / 1, 12 -> 11 |
| framework_mobo_lefthalf | 0 / 0 | 4 | yes / yes | 1090 / 1092 | 101 / 101 | 605 / 632 | 309 / 93 / 1, 4 -> 3 |
| MIDAS-MK1.1 | 7 / 7 | 17 | no / no | 3069 / 3069 | 234 / 234 | 266 / 288 | 563 / 26 / 0, 17 -> 17 |
| OpenFC | 1 / 1 | 2 | no / no | 1267 / 1267 | 163 / 163 | 445 / 363 | 445 / 173 / 0, 2 -> 2 |
| reCamera | 4 / 4 | 10 | no / no | 355 / 359 | 83 / 79 | 313 / 329 | 792 / 215 / 3, 10 -> 5 |
| spell_tome_bottom | 0 / 0 | 0 | yes / yes | 264 / 264 | 9 / 9 | 27 / 27 | complete |
| eternal-keypad | 0 / 0 | 0 | no / no | 2233 / 2233 | 34 / 34 | 102 / 102 | complete |
| urchin | 0 / 0 | 0 | yes / yes | 2571 / 2571 | 39 / 39 | 55 / 55 | complete |
| Blue_Line | 0 / 0 | 0 | yes / yes | 239 / 239 | 26 / 26 | 5 / 5 | complete |
| sum | 22 / 22 | | 6 / 6 | 17561 / 17567 | 1243 / 1316 | 2933 / 2937 | |

(x1691: WLED_Matrix 4 / 4 unconnected, 148 / 153 vias, 1311 / 1317 mm,
descent 2 kept, 8 -> 6; sums 18 / 18, passes 6 / 6.)

Every changed row: the six boards complete after the first route get no
descent and are identical. On the other six the classic moves after the
descent had the moves' budget less the descent's work, and the final
ladder the rest:

- USB_Keypad, MIDAS, OpenFC: nothing kept; the coupled loop's moves
  (6, 7, 9 trials) and the board are as off, only the work moves (the
  descent's 22-45 s; OpenFC's final ladder had 363 s instead of 409 s
  and used less).
- framework_mobo_lefthalf: D3 nudged (4 to 3 in the router), the
  coupled loop kept its 5 moves as off, the board passes both ways
  (HPWL +2 mm, work +27 s).
- reCamera: R23, U2, C26 moved (10 to 5 in the router); the coupled
  loop then kept 2 of 10 trials instead of 3 of 13; KiCad 4 both, vias
  83 to 79, HPWL +4 mm.
- WLED_Matrix: RGB23 nudged (12 to 11); KiCad 8 both; the board kept on
  the on side is the one with the pours connected (271 vias), off the
  final ladder's with the pours as tracks (194 vias), equal in KiCad's
  count.

No short-tier board gains or loses an unconnected item or a pass.

## Reading

- The trial is cheap now; the mechanism Florian asked for works as a
  mechanism: the descent finds moves the router confirms, on boards the
  coupled loop never touched (up to 11 kept on DSP and 4in1, the router's
  open count down 15-30 %).
- What it improves is the router's state of the first route. On the
  dense boards the final board comes from a route from scratch (the
  narrow-signal attempt, the ladder), which gains little from a few
  nudged parts and loses what the descent spent; where no final route
  runs (SNSP), the descent's board itself is judged, and KiCad finds it
  worse than the router's count says.
- Tier one (the incremental global route) is no ranking for millimetre
  moves: kept 4 % of the moves it predicted better, 6 % of those it
  predicted equal. It is only a cheap filter.

What would have to change for the descent to pay, in the order the data
suggests: give the final route the descent's routes as its start (resume
from the descent's router state instead of from scratch, as the first
route's resume does on boards not far from complete); take the
descent's work from the first route's negotiation rather than from the
last route; check the descent's board by KiCad's connectivity (or the
pour model's) before keeping it in place of the first route; and rebuild
only the changed tiles of the static maps in `Router::update` (more than
half of a DSP trial).

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
boards. Branch `move-descent`; snapshot x1691 (on main a49136c).

**Verdict: VERDICT_PLACEHOLDER**

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

RESULTS_PLACEHOLDER

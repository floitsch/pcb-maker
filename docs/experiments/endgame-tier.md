# The endgame tier (2026-11-24)

The strict passes are decided by the last few connections on about 22
boards (build/scratch/fundamentals/fundamental-approaches.md, section 1),
and every experiment there used to cost full layouts. The endgame tier
(`run.py --tier endgame`; how it works and how to run it:
[benchmarks/agent-tasks/README.md](../../benchmarks/agent-tasks/README.md#the-endgame-tier---tier-endgame))
resumes the nightly's stalled routed boards with their copper kept and
runs only the endgame: `route-kicad-board` with `existing_tracks: keep`
and 30 s of ladder work, then `repair-kicad-opens --local`.

## Inputs (`build/endgame-tier/v1`, `endgame.json`)

Every row with 1 to 7 KiCad-unconnected items that fails the strict rule:

| Source | Entries | Unconnected before | Role |
| --- | ---: | ---: | --- |
| sweep of 2026-11-18 (3b27f2d, layouts) | 20 | 63 | the tier |
| designer tier of 2026-11-24 (96e891b, the designer's placement) | 18 | 36 | the tier |
| designer tier of 2026-11-18 (3b27f2d) | 20 | 47 | reserve (`--only`): the prediction check |

Excluded from the tier as the input's own: SUMEC (its rules leave pads no
track can enter; the designer's board leaves GND, SHLD and +12V open too)
and Hub (the designer's own board leaves the same two GND zone pieces
open), checked with `drc-kicad-board` on the designers' boards
(`build/endgame-tier/v1/designer-drc`). Both are reserve entries of the
2026-11-18 designer tier for the prediction check.

## What resuming can and cannot do

Every path that resumes a routed board keeps its copper fixed:
`existing_tracks: keep` (route mode's default) makes the board's tracks
and vias fixed copper whose connections count as made; `--reuse-routes`
writes earlier copper back as the same kept copper; the local open repair
only adds copper. Nothing rips up a kept track, and the trouble repair
refuses an exchange that meets fixed copper. So the kept tier asks "can the
open connections be routed around the stalled copper". The layout's own
post-route repair for opens (`board_layout/additive_repair.rs`) is the
same search as `repair-kicad-opens --local`; a layout skips it as
`insufficient_remaining_work` when the layout spent its budget. The other
post-route stages do not touch opens (the via detour runs at 0 open; the
terminal launches and the plane escape only with budgets that are off).
None of these repairs were in the binary of the 2026-11-18 nightly.

`--endgame-release MM` cuts a window instead: the tracks and vias within
MM of each open item are removed before the resume, and the pieces of
kept copper left on no pad become terminals of their net.

## Baseline (main 96e891b, 30 s of ladder work, 3 x 2)

| | base-a | base-b |
| --- | ---: | ---: |
| unconnected (38 boards, 99 before) | 81 | 81 |
| boards that closed some / opened some | 11 / 0 | 11 / 0 |
| closed by the resumed route / by the local repair | 17 / 1 | 17 / 1 |
| strict passes | 3 | 3 |
| work, summed | 815 s | 834 s |
| wall of the run (summed over boards) | 9972 s (4799 s) | 722 s (2068 s) |
| CPU | 4608 s | 2370 s |
| rows cut by the ladder's wall guard | 4 | 0 |

- The three passes: capybully@sweep-1118 (3 to 0),
  poly_corne_split42_right@designer-1124 and DSP-ADAU1452@designer-1124
  (1 to 0). The sweep's 20 boards go 63 to 50, the designer tier's 18 go
  36 to 31.
- **Cost.** base-b took 12 minutes of wall at 3 x 2 with the load near 19,
  2370 CPU s. base-a ran beside the late nightly and other packages (load
  20 to 60): the memory ledger held boards back and the wall-clock guard
  cut four ladders, which tripled the wall.
- **Determinism.** The final unconnected count is identical on all 38
  boards. On the 34 boards neither run cut, vias, length, work and the
  strict verdict are identical too. The four rows base-a's wall guard cut
  (USB_Keypad, WLED_Matrix and protoesp of the sweep, split72_left) differ
  in work and protoesp in three vias, not in the count. Run it on a quiet
  machine, or compare only uncut rows.
- **The local repair** closed one item (WLED_Matrix@sweep-1118, 2 to 1) on
  the 35 boards it ran on; everything else the resumed route closed.

## The release variant (1.5 mm)

On the same boards at the same 30 s, `--endgame-release 1.5` is a large
loss: 86 to 413 unconnected on the 34 boards with a result, 4 boards
closed some, 27 opened some, 4 strict passes (the baseline's three less
none, plus reCamera@sweep-1118, 4 to 0). Two causes:

- A bug, fixed after this run: removing a board's last item dropped the
  board's closing parenthesis (4 boards ended `unterminated list`).
- The budget: the window frees tens of nets (vaio_re@designer-1124: 26
  nets open, 57 rerouted; protoesp@sweep-1118: 202 segments, 10 vias,
  32 nets), and 30 s of ladder work split over three attempts stops the
  negotiation after two iterations (`negotiation_work_limit`).

At four times the budget (120 s), against the kept tier at the same budget: pending (queued behind the load gate on 2026-11-24).

A cut through stored copper is a poor stand-in for a window solve: every
crossing becomes a terminal the router has to reach again, at the cut,
from a fixed stub. Proposal A's window solve has to live inside the router
(terminals at the window's boundary nodes, the rest fixed), where the
lattice already knows what crosses the window.

## Does the tier predict full runs?

**The designer tier, same placement, two binaries.** The designer tier
routed the designers' placements on 2026-11-18 (3b27f2d) and 2026-11-24
(96e891b). The tier resumes the 2026-11-18 boards (the reserve) with each
binary:

| Board (designer tier) | before | tier, 11-18 binary | tier, 11-24 binary | full route 11-24 |
| --- | ---: | ---: | ---: | ---: |
| reCamera_S101_v1.1 | 4 | 4 | **0, strict** | **0, strict** |
| poly_corne_split42_right | 2 | 2 | 2 | 1 |
| Hub | 2 | 2 | 2 | 3 |
| Sisu_SSE-9 | 1 | 3 | 3 | 2 |
| osprey_rev_a | 1 | 1 | 0, strict (cut) | 1 |
| DSP-ADAU1452 | 1 | 0, strict | 0, strict | 1 |
| RP2040_base | 7 | 6 | 6 | 7 |
| sbc_allwinner_a13 module | 3 | 2 | 2 | 3 |
| the other 12 | 26 | 26 | 26 | 26 |
| **all 20** | **47** | **46** | **41** | **44** |

- The one change between the binaries that is an endgame change, the
  corridors to pads walled in by their own footprint (cf6afbe1, reCamera's
  MIC1), shows on the tier as on the full route: 4 to 0 and a strict pass
  with the new binary only.
- The changes of the full route that come from the negotiation (corne
  right 2 to 1, Hub 2 to 3, Sisu 1 to 2) do not show: the tier starts
  after the negotiation. Sisu opens 1 to 3 on both binaries, where the
  resumed route cut its GND pour on F.Cu into pieces.
- The resume closes items the full route does not (DSP-ADAU1452, RP2040,
  the a13 module) on both binaries: more attempts on stalled copper.
  These gains are the tier's own, so judge a change as the difference
  between two binaries on the tier, not against the full run.
- osprey closes only on the new binary, in a row its wall guard cut;
  the full route of 2026-11-24 did not close it.

**The sweep.** The tier resumes the 2026-11-18 layouts with the 11-24
binary; the full sweep of 2026-11-24 (v46, the same binary, 73 strict)
laid them out again, with placements that changed between the nights:

| Board (sweep) | before | tier | full sweep 11-24 |
| --- | ---: | ---: | ---: |
| capybully | 3 | 0, strict | 0, strict |
| WLED_Matrix | 5 | 1 | 0, strict |
| reCamera_S101_v1.1 | 4 | 1 | 0, strict |
| osprey_rev_a | 2 | 1 | 0, strict |
| katia | 3 | 2 | 3 |
| Sisu_SSE-9 | 4 | 3 | 4 |
| USB_Keypad | 2 | 2 | 0 (not strict) |
| MIDAS-MK1.1 | 3 | 3 | 1 |
| vaio_re, SNSP-CPU-01, split72_left, the a13 module | 1, 2, 1, 5 | unchanged | 5, 9, 3, 8 |
| the other 8 | 28 | 28 | 28 |

The full sweep gained four strict passes on these boards. The tier
called one (capybully) and moved the other three the right way (WLED,
reCamera, osprey to one item each). It called no pass the full sweep
missed, and it does not see the layouts that got worse. Since the
placements changed, this is agreement in direction, not a test of the
endgame alone.

## When to refreeze

The tier's boards are the stalled results of the binary each entry
names. The nightly of 2026-11-24 has 17 near misses in its sweep; two
are not in the tier (0xCB-1337 pcb-panel, RP2040_base), and four of the
tier's 2026-11-18 sweep boards pass there. The 2026-11-18 inputs stay
valid as the endgame's inputs; freeze a v2 from a newer sweep
(`endgame_freeze.py build/endgame-tier/v2 ...`) when main's endgame
changes what these boards stall on.

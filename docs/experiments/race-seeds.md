# The race's allocation: placements against router seeds (2026-11-26/27)

[noise-sources.md](noise-sources.md) found that a first route's variance is
mostly the interaction of placer seed and router seed, and
[fundamentals.md](fundamentals.md) (headline 3, follow-up 4) asked what
follows for the race ([placer.md](../placer.md), "The race"): at an equal
work budget, is the race's one probe per placement the right way to spend
its probes, or should some of them be router seeds? This note measures the
ceiling of any selection change from stored layouts, builds a ground-truth
matrix (every candidate placement routed to the end under three router
seeds) and simulates the allocations on it.

Scripts in `experiments/race-seeds/`: `collect.py` (stored layouts into one
table), `ceiling.py`, `probe_rank.py`, `matrix.py` (the matrix's runner),
`simulate.py` (its checks, the allocations, the variance table). Data under
`build/scratch/race-seeds/` (not in git). Main is 592c6331.

## Verdict

- **No allocation beats today's race; nothing changes.** On the matrix
  (8 boards, 69 finished layouts) today's race (the placements at one
  router seed, the pick by the ripped count) ends with 220.7 unconnected
  items. Every allocation that spends probes on router seeds ends worse,
  even with its extra probes uncharged: one placement under three seeds
  253.0 (+15 %), two placements under two seeds 233.7 to 238.7, every
  placement under every seed (nine probes) 230.0, today's pick continued
  from its best probe of three seeds 233.0, a halving race with two first
  routes 232.0. None gains a strict pass: no cell of the matrix passes
  strictly.
- **Why: a probe's router-seed luck does not last to the end.** Within one
  placement, the ripped count of its probes under three router seeds does
  not rank the three finished layouts (Spearman -0.07 over 23 placements:
  9 positive, 12 negative). Across placements at one router seed it does,
  weakly (+0.25 over 21 races). So the probe says a little about the
  placement and nothing about the router seed, and "the best of m probes of
  one placement" picks at chance while giving up placements.
- **The finished layout depends on the placement more than the first route
  does.** Among one race's candidates the placement is 14 to 90 % of the
  final result's variance on seven of the eight boards, the router seed's own
  effect 0 to 39 %. The race's value is real: it ends 6 % below routing the
  first-ranked placement and 10 % below a chance pick.
- **The ceiling is there, but no probe reaches it.** An oracle that knew
  each candidate's finished layout at the race's router seed would end at
  198.0 (-10 %); one that also chose the router seed at 180.0 (-18 %). Over
  placer seeds (stored layouts, 25 sweep boards): the mean of three seeds
  724.3, the best of three 596 (-18 %), one strict pass to win (12 against
  11.0); on the short tier 3.0 against 1 and 1.7 strict passes. Even the
  first route's open terminals, known only after a whole first route, pick
  no better than today (218.7, -1 %).
- **Recommendation:** leave the race as it is. What is left to win in the
  selection needs a predictor of the finished layout that the probes'
  counts are not; more probes of the same kind do not buy it. The spread
  over placer seeds (18 %, about one strict pass in 25 boards) can be had by
  running the tool again at another placer seed, which an agent can do
  today; it is not an allocation inside one budget.

No code changed. The experiment's hook (`PCB_RACE_KEEP=<rank>`: the probe at
that rank of the race routes on) is commit 2228b66e of branch `race-seeds`
and is removed again at the branch's head; reproducing the matrix needs a
binary built at that commit.

## 1. The ceiling, from stored layouts

`collect.py` reads 662 stored layouts: the 2026-11-24 to 11-26 nightlies'
sweeps (109 boards each) and keep-arrangement's 335 (main cb7743eb and its
variants at placer seeds 1, 101, 201). A layout stores a finished result
only for the placement its race kept, so the stored data cannot say how the
race's ranking of its candidates relates to their finished layouts; that is
what the matrix of section 2 is for. What the stored data gives:

**The spread over placer seeds** (`ceiling.py`; main cb7743eb, seeds 1, 101,
201; "least" is an oracle that picks among the three finished layouts, which
costs three budgets):

| set | boards | unconnected per seed | mean | least | to win | strict per seed | boards strict on some seed |
| --- | ---: | --- | ---: | ---: | ---: | --- | ---: |
| short tier | 12 | 2, 3, 4 | 3.0 | 1 | 2.0 (67 %) | 10, 10, 8 (mean 9.3) | 11 |
| sweep subset | 25 | 775, 682, 716 | 724.3 | 596 | 128.3 (18 %) | 12, 10, 11 (mean 11.0) | 12 |

The sweep subset's failing boards (`!`: not a strict pass):

| board | s1 | s101 | s201 | mean | least |
| --- | ---: | ---: | ---: | ---: | ---: |
| PolyKybd 4x4 | 153! | 184! | 152! | 163.0 | 152 |
| Batch_1 | 137! | 114! | 107! | 119.3 | 107 |
| OpenESC 4in1-mini | 98! | 72! | 119! | 96.3 | 72 |
| OpenRX panel | 71! | 79! | 98! | 82.7 | 71 |
| OpenESC 4in1 | 81! | 54! | 77! | 70.7 | 54 |
| glasgow | 81! | 79! | 72! | 77.3 | 72 |
| MegaDrive | 54! | 18! | 35! | 35.7 | 18 |
| SmartSpin2k | 51! | 40! | 24! | 38.3 | 24 |
| eurorack-pmod | 18! | 16! | 6! | 13.3 | 6 |
| laptop power | 9! | 8! | 13! | 10.0 | 8 |
| RP2040_base | 11! | 5! | 2! | 6.0 | 2 |
| SNSP-CPU-01 | 8! | 8! | 8! | 8.0 | 8 |
| protoesp | 3! | 4! | 2! | 3.0 | 2 |
| WLED_Matrix | 0 | 1! | 1! | 0.7 | 0 |
| jiran-ble-lite | 0 | 0! | 0 | 0.0 | 0 |

So a perfect pick among finished layouts has 18 % of the open items and
about one strict pass to win: above the 5 % below which this study would
have stopped. No failing board of the subset passes at any seed except the
two that pass at seed 1 already.

**What a layout's early counts say about its end** (`probe_rank.py`; within
a board over its layouts, boards whose results differ by at least 3;
Spearman with KiCad's unconnected items):

| layouts | predictor | boards | mean | boards > 0 | < 0 |
| --- | --- | ---: | ---: | ---: | ---: |
| main cb7743eb, 3 placer seeds | the kept probe's ripped count | 10 | +0.00 | 5 | 5 |
| | the first route's open terminals | 10 | +0.45 | 8 | 2 |
| every stored layout (binaries mixed) | the kept probe's ripped count | 14 | +0.36 | 11 | 3 |
| | the first route's open terminals | 14 | +0.40 | 12 | 2 |

Three points a board is little, and the mixed row's correlation is partly
the binaries' (keep-arrangement's losing variants probe and end worse).

**How stable the race's ranking is under the router seed** (the noise
study's 48 E3 cells, the same candidates under four router seeds): the
ripped counts of two router seeds rank the candidates at a mean Spearman of
+0.18, and name the same best candidate in 34 of 72 pairs (chance is a third
to a half with three or two candidates).

**Determinism** (it makes the stored rows reusable): the 2026-11-26
nightly's sweep rows equal keep-arrangement's seed-1 rows of the same binary
on all 25 boards (unconnected, first route, placement digest).

## 2. The ground-truth matrix

Eight boards, fixed before any result: the noise study's three (protoesp,
OpenESC 4in1-mini, glasgow) and five failing boards of the 2026-11-26 sweep
with 1 to 20 unconnected items and three probes, one per repository, the
most open first (eurorack-pmod 18, RP2040_base 11, SNSP-CPU-01 8, Quanta75
6, link 5).

Per board at placer seed 1 (one pool of candidates: the race probes three,
on protoesp two), every candidate is routed to the end as a full layout
(tasks v7, the sweep's settings, 1500 s of work, two threads) under router
seeds 1001, 1002 and 1003 (`PCB_ROUTER_SEED`: the probes' and the first
route's net order and history), `PCB_RACE_KEEP` forcing the race's pick. A
cell's layout is exactly what the tool would have run had its race picked
that candidate: all probes charged, the kept probe routed on. Each cell also
reports every candidate's probe under its router seed. 69 cells, binary
`build/bin/pcb-maker-race-seeds` at 2228b66e.

Checks (`simulate.py`): every cell exited 0 with a KiCad result and the
forced pick; a probe's counts are the same in the three cells that ran it
(the placements and probes do not depend on which one routes on); no wall
guard fired, no check timed out, no verdict is cut. The machine was loaded
(1-minute load up to 94 from other sessions, runs started only below 30);
four cells whose wall guards were stretched to their cap (4.0) were run
again on a quiet machine and two more with a stretch of 3.4 and 3.7 were
run a second time: all six reproduce their rows exactly (unconnected, first
route, work). The work of a cell is 1111 to 1493 s of the 1500.

Cells: KiCad's unconnected items / the probe's ripped count / the first
route's open terminals. No cell is a strict pass.

| board | placement | r1001 | r1002 | r1003 |
| --- | ---: | --- | --- | --- |
| protoesp | 0 | 5 / 157 / 33 | 4 / 73 / 13 | 4 / 28 / 12 |
| | 1 | 2 / 40 / 11 | 3 / 112 / 14 | 2 / 23 / 8 |
| 4in1-mini | 0 | 92 / 521 / 145 | 100 / 503 / 157 | 95 / 464 / 144 |
| | 1 | 129 / 524 / 189 | 116 / 521 / 184 | 116 / 505 / 180 |
| | 2 | 92 / 497 / 168 | 89 / 457 / 169 | 87 / 503 / 170 |
| glasgow | 0 | 84 / 559 / 86 | 80 / 649 / 87 | 72 / 664 / 95 |
| | 1 | 70 / 660 / 100 | 78 / 680 / 87 | 68 / 687 / 120 |
| | 2 | 73 / 694 / 103 | 77 / 645 / 124 | 73 / 616 / 131 |
| eurorack-pmod | 0 | 25 / 287 / 39 | 26 / 255 / 44 | 22 / 246 / 33 |
| | 1 | 16 / 80 / 32 | 14 / 177 / 38 | 14 / 228 / 34 |
| | 2 | 19 / 326 / 38 | 17 / 339 / 43 | 29 / 282 / 36 |
| RP2040_base | 0 | 14 / 238 / 27 | 13 / 440 / 14 | 13 / 486 / 14 |
| | 1 | 11 / 429 / 11 | 14 / 232 / 23 | 6 / 427 / 17 |
| | 2 | 13 / 447 / 17 | 11 / 422 / 12 | 6 / 444 / 13 |
| SNSP-CPU-01 | 0 | 2 / 720 / 26 | 1 / 658 / 17 | 20 / 609 / 28 |
| | 1 | 20 / 702 / 58 | 31 / 758 / 43 | 25 / 617 / 54 |
| | 2 | 36 / 704 / 50 | 40 / 681 / 48 | 34 / 680 / 63 |
| Quanta75 | 0 | 3 / 250 / 13 | 4 / 119 / 8 | 1 / 148 / 13 |
| | 1 | 9 / 205 / 11 | 2 / 101 / 12 | 4 / 179 / 11 |
| | 2 | 2 / 179 / 15 | 8 / 229 / 20 | 4 / 73 / 11 |
| link | 0 | 10 / 812 / 10 | 6 / 828 / 10 | 8 / 830 / 11 |
| | 1 | 7 / 893 / 8 | 3 / 823 / 7 | 2 / 916 / 11 |
| | 2 | 1 / 902 / 4 | 1 / 422 / 2 | 3 / 809 / 8 |

Where the variance comes from (two-way random effects without replication,
a cell is deterministic so the interaction is the residual; components
floored at 0; 2 x 2 or 1 x 2 degrees of freedom a board, so each share is
rough):

| board | final: placement | router seed | interaction | sd of a cell | mean | first route: placement | router seed | interaction |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| protoesp | 79 % | 0 % | 21 % | 1.5 | 3.3 | 12 % | 2 % | 86 % |
| 4in1-mini | 90 % | 0 % | 10 % | 17.0 | 101.8 | 92 % | 0 % | 8 % |
| glasgow | 22 % | 30 % | 49 % | 5.5 | 75.0 | 53 % | 20 % | 27 % |
| eurorack-pmod | 46 % | 0 % | 54 % | 6.2 | 20.2 | 19 % | 59 % | 22 % |
| RP2040_base | 14 % | 39 % | 48 % | 3.4 | 11.2 | 0 % | 0 % | 100 % |
| SNSP-CPU-01 | 77 % | 0 % | 23 % | 15.9 | 23.2 | 84 % | 10 % | 6 % |
| Quanta75 | 0 % | 0 % | 100 % | 3.3 | 4.1 | 6 % | 0 % | 94 % |
| link | 69 % | 5 % | 26 % | 3.6 | 4.6 | 63 % | 24 % | 13 % |

The placement's share is larger here than the noise study's 0 to 30 %. The
two are different comparisons: there the rows were placer seeds, each
through its own race (the race's pick moved with the router seed and counted
as router variance), and the metric was the first route; here the rows are
one race's candidates, held fixed, and the metric is the finished layout.
The candidates of one race differ more than the kept placements of two
placer seeds do (SNSP-CPU-01: 1 to 20 against 20 to 40; the 4in1-mini's
second candidate 116 to 129 against 87 to 100).

The probe's ripped count against the finished layout (Spearman):

| over | cases | mean | > 0 | < 0 |
| --- | ---: | ---: | ---: | ---: |
| the placements of one race (one router seed) | 21 | +0.25 | 13 | 7 |
| the router seeds of one placement | 23 | -0.07 | 9 | 12 |
| a board's cells | 8 | +0.17 | 5 | 3 |
| the first route's open terminals, a board's cells | 8 | +0.31 | 6 | 2 |

## 3. The allocations simulated

`simulate.py` picks, per allocation, the cell it would have routed to the
end, from the probes' ripped counts alone, and reads that cell's result.
Where an allocation draws router seeds, every draw the matrix has is
averaged (today's: the three seeds; two of three seeds: the three pairs).
Today's pick is `race_pick` (the lowest ripped count where it is 2 % below
the first-ranked placement's). An allocation with more than today's three
probes would have to pay for them from the final route (a probe is up to
75 s of the 1500 s); that is not charged, so those rows are upper bounds on
what they gain. The halving row uses the first route's open terminals as its
long probe and would pay a whole extra first route.

| allocation | probes | unconnected, 8 boards | against today | strict passes |
| --- | ---: | ---: | ---: | ---: |
| (a) n placements x 1 seed, today | 3 | 220.7 | | 0 |
| no race: the first-ranked placement | 1 | 234.7 | +6 % | 0 |
| a chance pick among the placements | | 243.4 | +10 % | 0 |
| (b) 1 placement x 3 seeds, the best probe routes on | 3 | 253.0 | +15 % | 0 |
| (b) 2 placements x 2 seeds, the best probe | 4 | 233.7 | +6 % | 0 |
| (b) 2 placements x 2 seeds, the lower mean, its better probe | 4 | 238.7 | +8 % | 0 |
| (b) n placements x 3 seeds, the lower mean, its best probe | 9 | 230.0 | +4 % | 0 |
| (b) n placements x 3 seeds, the best probe | 9 | 230.0 | +4 % | 0 |
| (c) today's pick, routed on from its best probe of 3 seeds | 5 | 233.0 | +6 % | 0 |
| (d) halving: the two best probes' first routes, the better goes on | 3 + a first route | 232.0 | +5 % | 0 |
| oracle: the best placement at the seed | | 198.0 | -10 % | 0 |
| oracle: the best seed of the first-ranked placement | | 211.0 | -4 % | 0 |
| oracle: the best cell | | 180.0 | -18 % | 0 |
| oracle: the best first route at the seed | 3 first routes | 218.7 | -1 % | 0 |

Per board (unconnected; today's is the mean over the three seeds):

| board | today | 1 x 3 seeds | 2 x 2 best | n x 3 best | (c) | (d) | best placement | best cell |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| protoesp | 2.7 | 4.0 | 2.0 | 2.0 | 2.7 | 2.7 | 2.3 | 2 |
| 4in1-mini | 92.0 | 95.0 | 96.7 | 89.0 | 91.0 | 95.7 | 89.3 | 87 |
| glasgow | 79.0 | 84.0 | 82.7 | 84.0 | 80.3 | 78.7 | 71.7 | 68 |
| eurorack-pmod | 14.7 | 22.0 | 15.3 | 16.0 | 16.0 | 17.3 | 14.7 | 14 |
| RP2040_base | 11.3 | 14.0 | 14.0 | 14.0 | 14.0 | 9.3 | 9.3 | 6 |
| SNSP-CPU-01 | 13.7 | 20.0 | 13.7 | 20.0 | 21.7 | 19.0 | 7.7 | 1 |
| Quanta75 | 2.7 | 4.0 | 1.7 | 4.0 | 3.3 | 5.7 | 1.7 | 1 |
| link | 4.7 | 10.0 | 7.7 | 1.0 | 4.0 | 3.7 | 1.3 | 1 |

Reading it:

- The allocations with router seeds lose on four to eight of the eight
  boards each. SNSP-CPU-01 is the largest single difference (the
  first-ranked placement's best probe is the seed that ends at 20, against
  2 and 1); without that board the nine-probe rows and (c) are at +1 to
  +2 % of today's, still not better, and still uncharged.
- The differences among the allocations are within what eight boards can
  resolve: the simulation does not show that today's is the best, it shows
  that none of the others wins, clearly or at all. By the rule set before
  the run (only a clear winner is implemented) nothing is.
- Today's race is worth keeping: -6 % against no race, -10 % against a
  chance pick, and its largest loss (SNSP-CPU-01 at one seed, where a
  2.5 % margin moved the pick to a placement that ends at 20 instead of 2)
  is one board.
- The oracle rows bound what a better predictor at the same probes could
  win: 10 % by the placement, 18 % with the router seed. No count the
  probes or the first route produce gets there.

## What is not verified

- Eight boards, one placer seed, three router seeds: the simulation's rows
  differ by less than its resolution, and one board moves the totals by
  several percent. It supports "no allocation wins", not a ranking of the
  losers.
- The matrix has no strict pass in any cell, so it says nothing about
  strict passes won or lost by an allocation, only that none was there to
  win on these boards with these candidates.
- Shorter or longer probes were not run: (d) is approximated by the first
  route, and the equal-budget cost of the extra probes in (b) and (c) is not
  modelled (it could only make them worse).
- Step 3 of the plan (the winner judged on the short tier and the 25 sweep
  boards) did not run: there is no winner. Whether keep-arrangement's stored
  rows of main cb7743eb reproduce on today's main was therefore not checked.
- The oracle's 18 % over placer seeds (section 1) is on main cb7743eb, not
  on today's main.

## Reproducing

```sh
experiments/race-seeds/collect.py build/scratch/race-seeds/stored.json \
    build/scratch/keep-arrangement build/nightly/2026-11-2{4,5,6}/sweep
experiments/race-seeds/ceiling.py build/scratch/race-seeds/stored.json --boards
experiments/race-seeds/probe_rank.py build/scratch/race-seeds/stored.json
# A binary built at 2228b66e (PCB_RACE_KEEP) as build/bin/pcb-maker-race-seeds:
experiments/race-seeds/matrix.py run            # 69 layouts, 6 at once
experiments/race-seeds/simulate.py --cells --boards
```

The cells are in `build/scratch/race-seeds/matrix` (the four first runs at
the stretch cap in `matrix-cut`, the two repeats in `matrix-check`): 69
layouts at about 1300 s of work each, about 25 CPU hours.

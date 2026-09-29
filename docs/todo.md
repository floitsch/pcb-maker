# TODO

Open items that are not yet scheduled. Remove an entry when it is done or
turns out to be moot; record the outcome in the relevant doc.

The ranked work list from the failure analysis of 2026-09-25 is in
[failure-analysis.md](failure-analysis.md#work-list); it covers most of the
router and placer items.

## Benchmarks

- **Check the Freerouting setup on Multichannel.** The head-to-head in
  [benchmarks.md](benchmarks.md#head-to-head-with-freerouting-and-tscircuit-2026-09-22)
  reports 180 open for Freerouting on Multichannel, attributed to "via class
  below board minimum". That is more unconnected items than the board has
  nets (79) and looks like a broken setup rather than a routing failure.
  Verify the DSN export (via padstacks, class rules, board minimums) and the
  session import before the number is quoted anywhere, including the README.
  Artifacts: `build/corpus-fr7/multichannel/freerouting*`.
- **Breadboard fence issues.** [benchmarks/fence/README.md](../benchmarks/fence/README.md#issue-log)
  lists what went wrong on those boards, each with a benchmark: keepout zones
  inside footprints are ignored by the router (1), signals get routed on a
  `power` plane layer (9), no placement constraints (2, 11; proposed
  `constraints.json`), pin swapping (3; implemented, see
  [pin-swap.md](pin-swap.md), but on the fence it does not yet beat the
  hand-made assignment), no warning for overlapping fixed parts (5), and
  `free` vs `fixed_patterns` / overhanging footprints (10). The fence boards
  work around 1, 2, 3 and 9 in their inputs; the README says how to take each
  workaround out to test a fix.

## Router

- **The attempt ladder stops at an expensive clean rung.** On Interf-U with
  seeded runs ([GPU survey](reviews/2026-09-26-gpu-survey.md#1-seed-spread-of-the-existing-router)),
  the plane-skeleton rung comes out clean for 3 of 7 seeds with 68–82 vias
  and the ladder stops, while the pour-nets-as-tracks rung gives 24–31 vias
  whenever it runs. Continue to the next rung when a clean result has far
  more vias than the best unclean one, or rank clean rungs by vias.
- **Seeded portfolio.** Best of 7–15 seeds (`PCB_ROUTER_SEED`) saved vias on
  every board tried (PIC 2 → 0, Multichannel 18 → 14, Interf-U 28 → 24).
  Idle cores could run seeds in parallel and keep the best board.

## Placer

- **Alignment constraints.** A row of parts at a pitch (LED bars, key rows,
  test points) cannot be said yet. Best built as a macro body in the
  lowering: the row moves and turns as one part and is expanded back to
  its footprints after placement.
- **Parts as wide as the board.** On raspberry_pi_pullup (a 4.8 mm wide
  board) the parts' copper reaches both edges; the placer's grid cannot
  hit the 0.002 mm of room. Snapping to both edges at once would.
- **Crowded single sides.** FogDrive, HaveSome and tiny_tapeout leave a
  part unplaced with a side 78-93 % full. The hint tells the agent to move
  parts to the other side; the placer does not choose sides by itself.
- **Legalization loses 30-90 % of the global placement's wirelength.**
  Examples: PIC 678 → 1306 mm, hierarchy 493 → 907-1169 mm, Interf-U
  3206 → 4077 mm; measured 2026-09-26/28, graph-first experiment on branch
  `experiment/graph-first-placement`.
  - **Not the anneal's random walk alone.** A minimum-displacement overlap
    relaxation in its place (tried 2026-09-28, reverted) ends no better
    (hierarchy 974, Interf-U 5301).
  - **The global placement's wirelength is partly fictitious.** At the 4 %
    bin-overflow stop, parts still overlap physically: 64 bins smear small
    parts over about 1.5 mm.
  - **Exact pairwise overlap force: tried 2026-09-29, reverted.** This was
    the exact overlap of body pairs as an extra force in the late global
    iterations, which keep going until the pairs are nearly apart. Final
    wirelength did not improve consistently:
    - improved: hierarchy −11 % (weight 10), PIC −18 % (10), Interf-U
      −11 % (0.1);
    - worse: multichannel +14 % and sonde +18 % (10);
    - no weight is best on every board, and the changes are within the
      seed spread.
  - **Open candidates.** Bins sized from the median part; the survey's
    CP-SAT NoOverlap2D / sparrow legalizer
    ([algorithm survey](reviews/2026-09-26-algorithm-survey.md), section 5).
    Meanwhile three placement seeds (the best one kept) absorb part of the
    spread.
- **LNS endgame: tried, no gain (2026-09-28).** This was the survey's
  large-neighbourhood search: rip up an open net with every net in its
  window, reinsert with the open net first in a shuffled order, and keep
  the round if fewer terminals stay open.
  - Result on four PCBench "open" boards: rounds were kept, but no board
    got closer to complete.
  - Why: each ends with two or three nets that compete for the same gap,
    and only one fits on the lattice, whatever the order.
  - The designer fitted both, so the limit is geometric (lattice
    resolution or topology), not negotiation.
  - Reverted; the survey's exact window solving or a finer local lattice
    would be the next step for these.

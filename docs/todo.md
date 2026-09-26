# TODO

Open items that are not yet scheduled. Remove an entry when it is done or
turns out to be moot; record the outcome in the relevant doc.

## Benchmarks

- **Check the Freerouting setup on Multichannel.** The head-to-head in
  [benchmarks.md](benchmarks.md#head-to-head-with-freerouting-and-tscircuit-2026-09-22)
  reports 180 open for Freerouting on Multichannel, attributed to "via class
  below board minimum". That is more unconnected items than the board has
  nets (79) and looks like a broken setup rather than a routing failure.
  Verify the DSN export (via padstacks, class rules, board minimums) and the
  session import before the number is quoted anywhere, including the README.
  Artifacts: `build/corpus-fr7/multichannel/freerouting*`.

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


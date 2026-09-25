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

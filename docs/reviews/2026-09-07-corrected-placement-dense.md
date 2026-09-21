# Dense routing after the harmonic locality fix

Both corrected-harmonic placement probes complete nineteen connections using
ordinary routing alone. There are no guidance, rip-up or fallback attempts.
All 38 committed prefixes have zero ERC, design DRC, schematic parity and
selected-net open findings, native SVG previews, and unchanged component poses.
The full semantic poses are preserved by the declared placement stage; both
clearance profiles use the same native placement hash. Final metadata warnings
are twenty in each case.

| Corrected harmonic | Completed | Copper mm | Vias | ESP pad-gap crossings | A* states | Preflight states |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Blocked, 0.25 / 0.20 mm | 19/19 | 314.514 | 25 | 0 | 2,011,386 | 24,361,888 |
| Open, 0.15 / 0.10 mm | 19/19 | 316.530 | 23 | 1 | 2,246,920 | 22,284,231 |

These are the same nineteen signal connections used in the earlier dense study,
starting with zero copper. Full connectivity determines the placement, but the
routed prefix excludes LED_SERIES, VCC and GND. The separate LED probe validates
that local circuit change; neither study is a fully routed fifty-two-connection
board. The open profile changes width and clearance across all selected routes,
not only inside ESP pad gaps.

The historical harmonic reference used identical routing and template settings
and identical non-pose problem properties. It used an earlier executable, so
this is not an isolated binary comparison. At matching prefixes:

- Blocked, sixteen connections: corrected copper is 261.977 mm with 21 vias,
  versus historical 259.597 mm with 20 vias. The corrected run continues to
  nineteen without recovery; the historical ordinary run stopped at sixteen.
- Open, nineteen connections: corrected copper is 316.530 mm with 23 vias,
  versus historical 322.270 mm with 21 vias. Corrected total A* plus preflight
  work is 24,531,151 states, versus 27,244,648 historically.

The result supports keeping the constraint/weighting bug fixes. It does not
support claiming that every quality metric improves. Wider blocked rules can
produce a shorter final route than open rules in this bounded, sequential
search; more geometric options do not guarantee a better chosen solution.

The executable is frozen at SHA-256
`30cafe79fc35a056c97bfbc62b7c4cbb9e82de3fe5ec8f60b7e39cfac08551e5`.
The paired runner completed with exit zero. Concurrent wall times were about
742 seconds per case and are advisory, not a controlled performance result.
Summary auditing checks every native prefix, exact source pose preservation,
profile-pair pose hashes, copper measurements and actual pad-gap crossings.

The probe runner now accepts repeated `--seed NAME=DIRECTORY` arguments, so new
prepared placements can use this standard benchmark without editing the runner.
The summary and viewer handle arbitrary placement names and case counts.

- [Corrected routing playback](../../build/placement-locality-dense-2026-09-07/index.html)
- [Matched historical comparison](../../build/placement-locality-dense-2026-09-07/historical-comparison.html)
- [Measurements](../../build/placement-locality-dense-2026-09-07/summary.json)
- [Related orientation diagnostic](2026-09-07-orientation-counterfactuals.md)
- Reproduction: `experiments/placement-locality/README.md`

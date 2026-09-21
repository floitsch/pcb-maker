# Dense validation of production orientation refinement

Both refined placements complete all nineteen signal connections with ordinary
routing, without guidance or rip-up. All 38 committed prefixes pass native ERC,
design DRC, schematic parity and selected-net connectivity checks. All component
poses remain fixed during routing and every prefix retains its native preview.

| Profile | Final copper mm | Vias | Pad-gap crossings | A* expansions | Preflight expansions |
| --- | ---: | ---: | ---: | ---: | ---: |
| Blocked, 0.25 / 0.20 mm | 314.514 | 25 | 0 | 2,011,386 | 26,696,859 |
| Open, 0.15 / 0.10 mm | 316.530 | 23 | 1 | 2,246,920 | 24,456,246 |

The earlier unrefined corrected-harmonic runs have identical copper geometry
hashes and A* counts at **every matching prefix**, not merely the same final
lengths. Their preflight totals are lower: 24,361,888 and 22,284,231. Refined
preflight work increases by 9.58% and 9.75%, respectively. This is observed
search-state work, not a controlled CPU timing result. Inspection shows that
the current reachability traversal uses a stack; its explored count can be
sensitive to small obstacle changes even when A* chooses identical copper.

Non-pose problem data and routing/template configurations match exactly. Centers
are unchanged; eight component orientations differ. The two studies used
different executables, so this is not an isolated binary comparison. The dense
run froze the pre-net-identity-fix executable at
`ea403fddec48ec1b1333a6c65d9996fc56fd78df75788bc6dcebb9167e147a16`.
Later adapter work in this turn cannot affect these frozen results.

The result supports retaining orientation refinement as an optional placement
improvement: its separate matched LED probe shortens copper from 5.441 to
2.027 mm, while the nineteen-connection probe preserves routing completion and
copper quality. It does not establish a global routing speed benefit. These
nineteen signal connections exclude LED_SERIES, VCC and GND; full fifty-two-net
routing and the wider supply/ground branches remain outside this validation.

Both child runs terminated with exit zero; the runner and summary audit also
completed. Concurrent wall times were approximately 1080 seconds per case and
are advisory. Native metadata warnings are kept separately from design findings.

- [Routing playback](../../build/orientation-refinement-dense-2026-09-07/index.html)
- [Matched reference comparison](../../build/orientation-refinement-dense-2026-09-07/historical-comparison.html)
- [Per-prefix copper equivalence and work differences](../../build/orientation-refinement-dense-2026-09-07/prefix-equivalence.json)
- Reproduce with `experiments/placement-exploration/dense_probe.py --seed refined=DIRECTORY`.

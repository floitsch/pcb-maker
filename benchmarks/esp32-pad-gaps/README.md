# ESP32 pad-gap rule profiles

Two fixed-placement routing profiles preserve the same component geometry,
connectivity, placement policy, via dimensions, and per-case grid resolution.
They answer whether opening the ESP32 pad rows improves routing and how it
changes search work.

Latest paired cold run: both profiles complete 19/19 with native verification
using the updated grid builder and plated-terminal layer search. Blocked uses
474.282 mm of track, 24 vias and 3,035,410 expansions; open uses 477.701 mm,
18 vias and 5,017,180 expansions. Open uses seven pad-gap/net passages and
improves length plus 2 mm per via by 8.582 mm, with 65.3% more expansions.
[Paired evidence](../../build/esp32-pad-gap-layer-search-comparison-2026-09-07/validation.json),
[blocked preview](../../build/esp32-pad-gap-prefix19-blocked-layer-search-2026-09-07/final-preview.png),
and [open preview](../../build/esp32-pad-gap-prefix19-open-layer-search-2026-09-07/final-preview.png)
are retained. The result sections below preserve earlier experiments.

| Profile | Selected trace width | Copper clearance | Width + two clearances | 0.37 mm gap |
| --- | ---: | ---: | ---: | --- |
| blocked | 0.25 mm | 0.20 mm | 0.65 mm | unavailable |
| open | 0.15 mm | 0.10 mm | 0.35 mm | available |

[JLCPCB's official capabilities](https://jlcpcb.com/capabilities/pcb-capabilities/)
checked on 2026-09-07 list 0.10 mm minimum track width and spacing for 1 oz,
one- and two-layer rigid boards. Both native project templates explicitly set
the manufacturing minimum track width to 0.10 mm. Their copper-clearance
minimums and Default netclasses match the selected profile. The router chooses
0.25 or 0.15 mm respectively; a configured manufacturing minimum is not an
instruction to use that width everywhere.

## Cases

- `gap-*`: a focused crossing of two adjacent ESP32-sized pads, 1.5 × 0.9 mm
  on 1.27 mm pitch. Both terminals are on F.Cu. The blocked profile must avoid
  the gap; the open profile can cross it directly. All poses are fixed. These
  are representative adjacent pads, not a complete ESP32 circuit.
- `dual-prefix03-*`: the real dual-ESP32 benchmark, with all 43 components and
  its first three signal connections. This is the default board-level smoke
  comparison.
- `dual-prefix19-*`: the same pair extended to 19 signal connections, retained
  as larger cases. They are not part of the smoke corpus.
- `dual-prefix25-*` and `dual-prefix50-*`: denser cases, declared by
  `prefix25-physical-cost-corpus.json` and `prefix50-physical-cost-corpus.json`.
  Their cold process limits are 1,200 and 2,400 seconds respectively; each A*
  search retains its two-million expansion limit. The first experiments are
  [continuations from retained boards](../../docs/reviews/2026-09-07-dense-prefix-routing.md),
  not cold runs: both profiles reach 25, then stop at 27 blocked and 31 open
  on search-budget exhaustion. The source's VCC and GND nets follow these 50
  signal/auxiliary/control connections and are outside this configuration.

The focused case uses a common 0.02 mm grid. At 0.15/0.10 mm, the usable band
for the trace center is only 0.02 mm wide: coarser uniform grids can miss a
physically valid channel depending on their origin. Both dual profiles use
0.05 mm resolution. This can represent some pad gaps but does not guarantee
coverage of every 0.02 mm-wide band. Gap-aligned or adaptive routing remains an
implementation target; finer global grids are expensive.

A [bounded gap-alignment policy](../../docs/reviews/2026-09-07-pad-gap-grid-alignment.md)
is now available. `gap-alignment-corpus.json` runs a two-candidate portfolio
using the original grid and `grid_alignment: "nearest_narrow_pad_gap"`. Its
focused cases also use 0.05 mm spacing to expose missed narrow bands. The
program chooses the gap and shift from board geometry and skips an equivalent
grid when alignment makes no change. This alternate policy is opt-in; it does
not represent every gap simultaneously or implement local neck-downs.

```sh
target/release/pcb-maker benchmark-route-corpus \
  benchmarks/esp32-pad-gaps/gap-alignment-corpus.json build/pad-gap-alignment
python3 benchmarks/esp32-pad-gaps/summarize.py build/pad-gap-alignment
```

More legal choices do not imply more search expansions. A shortcut can reduce
work, while new dead ends can increase it. Compare completion, native findings,
physical copper length, vias, actual pad-gap crossings, search expansions, and
elapsed time separately. Matched grid resolution controls one major confounder;
wall time still includes native checks, process startup, and cache behavior.

## Running and checking

The existing comparison harness runs pcb-maker and the pinned Freerouting
reference, records native checks and process budgets, and creates source/result
renders. The verification hook also records `preview.svg` with each native
attempt. Required Freerouting/Java paths are in each `benchmark.json`.

```sh
cargo build --release --bin pcb-maker
# Use a fresh output directory for every experiment.
target/release/pcb-maker benchmark-route-corpus \
  benchmarks/esp32-pad-gaps/smoke-corpus.json build/esp32-pad-gaps-smoke
python3 benchmarks/esp32-pad-gaps/summarize.py build/esp32-pad-gaps-smoke
```

The summary script reads native PCB geometry through pcbnew. It checks matched
placement, detects straight F.Cu tracks crossing the midplanes between adjacent
ESP pad pairs, and requires the focused open case to cross its gap with native
acceptance while the blocked case does not. It writes `pad-gap-summary.json`
even when these checks fail. It reports unsupported front-layer arcs rather
than claiming complete crossing coverage. Native DRC establishes clearance;
the detector establishes whether the newly available passage was used.

Run either larger case independently with `benchmark-route-compare` and its
`dual-prefix19-*/comparison.json`. Both use the same finite 300-second pcb-maker
budget; timeout and completed-prefix length remain benchmark outcomes.
The `comparison-physical.json` variants allow 1,200 seconds each.

To compare separately launched runs without copying their artifacts, give the
summary command named source directories. It checks that the paired pcb-maker
executables and component placements match, and retains the source report paths:

```sh
python3 benchmarks/esp32-pad-gaps/summarize.py build/pad-gap-comparison \
  --case dual-prefix19-blocked=build/my-blocked-run \
  --case dual-prefix19-open=build/my-open-run
```

The 19-connection target is a signal-routing prefix. The complete template
declares 52 connections, including VCC and GND; passing this prefix does not
establish completion of the full circuit.

## Local neck-down target

The current native candidate chooses one width per connection. These cases
therefore measure a uniform narrower-signal baseline; they do not implement
local neck-downs. The intended extension is nominal-width routing in open space
with permitted short 0.10–0.15 mm sections between pads. That requires width to
be represented on individual segments and respected by routing, quality
measurement, shortening/repair, and native materialization. Keep both profiles
when adding it, and measure narrow-section length as well as total length.

The first run exposed a native-project mismatch: selecting 0.15 mm in the
router did not override KiCad's 0.20 mm minimum track-width constraint. The
semantic template now accepts explicit `minimum_trace_width_mm` and
`copper_clearance_mm`; omitted settings preserve prior defaults. Initial
rejected evidence is retained in `build/esp32-pad-gap-profiles-2026-09-07`.

## Verified first results

The corrected [run summary](../../build/esp32-pad-gap-profiles-native-rules-2026-09-07/pad-gap-summary.json)
passes all passage/placement checks. All four pcb-maker runs reach their targets
and pass native completion. These are single runs, not statistical timing claims.

| pcb-maker case | Profile | Committed | Length | Vias | Inter-pad passages used | Expansions |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| focused gap | blocked | 1/1 | 8.014 mm | 2 | 0 | 41,731 |
| focused gap | open | 1/1 | 8.014 mm | 0 | 1 | 401 |
| dual prefix 3 | blocked | 3/3 | 91.415 mm | 2 | 0 | 255,483 |
| dual prefix 3 | open | 3/3 | 91.357 mm | 2 | 0 | 269,718 |

The focused shortcut removes two vias and reduces search work. In the larger
placement the open profile increases expansions by about 5.6%, and pcb-maker
still chooses routes that do not traverse any pad gap. Thus the benchmark
records both legal availability and actual use; it does not assume the current
algorithm takes advantage of every available passage. The 19-connection pair
is configured but was not run in this experiment.

Previews: [focused blocked](../../build/esp32-pad-gap-profiles-native-rules-2026-09-07/gap-blocked/renders/pcb-maker-result-front.png),
[focused open](../../build/esp32-pad-gap-profiles-native-rules-2026-09-07/gap-open/renders/pcb-maker-result-front.png),
[dual blocked](../../build/esp32-pad-gap-profiles-native-rules-2026-09-07/dual-prefix03-blocked/renders/pcb-maker-result-front.png),
[dual open](../../build/esp32-pad-gap-profiles-native-rules-2026-09-07/dual-prefix03-open/renders/pcb-maker-result-front.png).
Back views and automatic SVG previews are retained alongside these.

The Freerouting reference uses an inter-pad passage in the open dual case.
Its dual-board outputs pass the base native completion check but are rejected
by the existing stricter source/result comparison of library-mismatch warning
records. The summary preserves both results separately; these runs establish
pcb-maker profile behavior and do not establish a competition winner. No
admission exception was introduced for this experiment.

Validation also includes 88 passing pcb-kicad library tests, Rust formatting,
and Python compilation. The summary checker correctly rejects the original
minimum-width-mismatch run, and accepts the corrected run.

## Physical via-cost variant

`physical-cost-corpus.json` keeps the original geometry/rules/grid pair and adds
`via_cost_mm: 2.0` in each `routing-physical.json`. Original integer-cost
configurations remain available for reproducing the first results above.

The native search now converts a requested straight-track-equivalent via
penalty to integer units as `ceil(via_cost_mm * straight_cost / resolution_mm)`.
Thus 2 mm is 800 units on a 0.25 mm grid, 4,000 on a 0.05 mm grid, and 10,000
on a 0.02 mm grid. Without the optional field, `via_cost` retains its historical
meaning. Non-finite, negative, or overflowing physical costs are rejected.
The same conversion feeds search requests and retained-tree path costing.
Other cost terms, including bends, retain their existing semantics.

The fixed 800-unit penalty in the initial 0.05 mm experiments represented only
0.4 mm, even though the outer quality objective penalized each via by 2 mm.
On the retained open dual-prefix second-connection input, changing only the
integer via cost from 800 to 4,000 produces a native-admitted zero-via route
through U2 pads 19/20. The physical-cost option preserves the specified
via-versus-length tradeoff when grid resolution changes. Grid coverage and
the selected route can still change. It is not a local neck-down implementation.

```sh
target/release/pcb-maker benchmark-route-corpus \
  benchmarks/esp32-pad-gaps/physical-cost-corpus.json build/esp32-pad-gap-physical-cost
python3 benchmarks/esp32-pad-gaps/summarize.py build/esp32-pad-gap-physical-cost
```

The [physical-cost run](../../build/esp32-pad-gap-physical-cost-2026-09-07/cost-comparison.json)
completes all four pcb-maker targets with native verification and passes every
gap/placement check. Each cold source PCB is byte-identical to its original
integer-cost counterpart. All 90 pcb-kicad library tests pass.

| Case | Length | Vias | Inter-pad passages used | Expansions |
| --- | ---: | ---: | ---: | ---: |
| focused blocked | 9.183 mm | 0 | 0 | 157,875 |
| focused open | 8.014 mm | 0 | 1 | 401 |
| dual prefix 3 blocked | 93.875 mm | 0 | 0 | 330,437 |
| dual prefix 3 open | 92.382 mm | 0 | 1 | 247,130 |

Both dual cases eliminate two vias and improve length plus 2 mm per via.
Against its original run, blocked search work rises from 255,483 to 330,437
expansions; open search work falls from 269,718 to 247,130. These observations
do not establish a general speedup from opening pad gaps. The focused blocked
case now prefers a longer same-layer detour to two vias. The reference admission
caveat above still applies, and the 19-connection variants remain unrun.

Automatic result previews: [focused blocked](../../build/esp32-pad-gap-physical-cost-2026-09-07/gap-blocked/renders/pcb-maker-result-front.png),
[focused open](../../build/esp32-pad-gap-physical-cost-2026-09-07/gap-open/renders/pcb-maker-result-front.png),
[dual blocked](../../build/esp32-pad-gap-physical-cost-2026-09-07/dual-prefix03-blocked/renders/pcb-maker-result-front.png),
[dual open](../../build/esp32-pad-gap-physical-cost-2026-09-07/dual-prefix03-open/renders/pcb-maker-result-front.png).

## Longer-prefix checkpoint

`extended-physical-cost-corpus.json` runs the blocked/open 19-connection pair.
These extended physical-cost comparisons now allow 1,200 seconds per pcb-maker
run; the smaller smoke comparisons retain their 300-second limits. The initial
300-second extended run below is preserved as a timeout observation, including
its original configuration snapshot.
The first standalone open-profile run is retained in
[`build/esp32-pad-gap-prefix19-open-physical-2026-09-07`](../../build/esp32-pad-gap-prefix19-open-physical-2026-09-07/diagnosis.json).
It reaches 6/19 before the declared 300-second router limit, with 1,417,196
expansions, 181.918 mm of track and four vias. All six committed transactions
pass native verification. This is partial-prefix validity, not completion of
the 19-connection target. The seventh transaction was interrupted; no search
failure was established. The blocked 19-connection case remains unrun.

The first three connections reproduce the smaller physical-cost run's 247,130
expansions. Across all six committed transactions, the retained diagnosis
separates route-generation time and candidate native-gate time from total
transaction time. Time outside route generation also includes materialization,
rendering and other orchestration. A larger time budget or a controlled
orchestration comparison is needed before diagnosing the cutoff as a geometric
routing limitation. No routing capability claim follows from this timeout.

The Freerouting result passes base native completion for all 19 connections,
but again fails strict admission on four changed library-warning records.
The comparison's reported tie therefore does not imply equivalent routing
outcomes. [Automatic partial-result preview](../../build/esp32-pad-gap-prefix19-open-physical-2026-09-07/renders/pcb-maker-result-front.png).

The admission mismatch is now [fixed in the benchmark and sequential router](../../docs/reviews/2026-09-07-routing-admission-metadata.md).
A [fresh 19-connection Freerouting run](../../build/esp32-pad-gap-prefix19-open-admission-fixed-2026-09-07/competitive-result.json)
passes both base verification and route admission while retaining 21 metadata
warnings. Its source matches the original except for UUID fields. The same four
introduced metadata warnings would still reject this new result under the old
comparison. Historical results above remain unchanged; pcb-maker's 6/19 timeout
has not been rerun in this admission experiment.

## Open-profile completion from the retained checkpoint

The [continuation](../../build/esp32-pad-gap-prefix19-open-continuation-2026-09-07/completion-summary.json)
starts from the timed-out run's last admitted six-connection board, after its
process had exited. It preserves the placement, connection order, routing
configuration and all committed copper, then automatically commits the remaining
13 connections. All 19 transactions have retained native verification and
automatic SVG previews; the final board has zero ERC, DRC design, parity and
selected-net connectivity findings. The 21 library metadata warnings remain
reported. No candidate was rejected and no routing budget was increased.

| Open prefix 19 outcome | pcb-maker continuation | Fresh Freerouting reference |
| --- | ---: | ---: |
| Connected targets | 19/19 | 19/19 |
| Track length | 481.290 mm | 503.436 mm |
| Vias | 18 | 6 |
| Length + 2 mm per via | 517.290 mm | 515.436 mm |

pcb-maker uses seven distinct pad-gap/net passages and 5,192,423 total search
expansions across the 19 committed transactions. The largest individual search
is connection seven at 1,212,119 expansions, below the configured two-million
limit. The continuation itself takes 531.2 seconds. Completed transactions from
both runs total 803.0 seconds, including 239.3 seconds of route generation and
242.7 seconds in candidate native gates. These totals exclude the interrupted
seventh attempt from the original run and are **not** a fresh cold-run timing.
Time outside these measured phases includes target verification, materialization,
rendering and other orchestration.

This establishes that the open profile can complete the longer target with the
existing search policy. The remaining questions are the blocked-profile result,
route-quality improvements (especially the length/via tradeoff), and measured
orchestration improvements. Inspection found repeated via and reverse-edge
geometry queries during grid construction. This work is now
[deduplicated and checked against the original calculation](../../docs/reviews/2026-09-07-grid-query-reuse.md),
with an identical retained ESP32 candidate and 43.8% lower median process CPU
time in a three-replay-per-version comparison. The existing deferred unrouted-target verification
option also offers a controlled experiment while retaining final admission.

[Final automatic preview](../../build/esp32-pad-gap-prefix19-open-continuation-2026-09-07/final-preview.png).

## Completed longer-profile comparison

The [blocked cold run](../../build/esp32-pad-gap-prefix19-blocked-physical-2026-09-07/competitive-comparison.json)
also completes 19/19, with no native rejection or search-budget failure. Both
pcb-maker and Freerouting pass the corrected route-admission policy in this
run. The blocked result uses no pad gaps. Its component-placement hash matches
the completed open-profile board exactly.

| pcb-maker profile | Connected | Length | Vias | Pad-gap/net passages | Expansions | Length + 2 mm/via |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| blocked | 19/19 | 478.272 mm | 24 | 0 | 3,269,656 | 526.272 mm |
| open | 19/19 | 481.290 mm | 18 | 7 | 5,192,423 | 517.290 mm |

The open profile adds 3.017 mm of track but removes six vias, improving the
declared score by 8.983 mm while increasing search expansions by 58.8%.
This supports retaining both profiles as a quality/search-work tradeoff. Width
and clearance change throughout each connection, so the comparison does not
isolate pad-gap availability from all other clearance effects.

Both results use the original grid builder, before the symmetric-query
optimization. The blocked cold run takes 660.4 seconds, partly alongside
compilation, native tests and replay measurements. The open result spans two
processes, so their wall times are not a controlled performance comparison.

[Machine-readable comparison](../../build/esp32-pad-gap-prefix19-blocked-physical-2026-09-07/profile-comparison.json)
and [automatic blocked-board rendering](../../build/esp32-pad-gap-prefix19-blocked-physical-2026-09-07/renders/pcb-maker-result-front.png)
are retained. The new grid-query optimization is separately verified with an
identical ESP32 candidate, native admission, a via-window regression, and
repeated route-generation CPU measurements.

A subsequent [local cleanup investigation](../../docs/reviews/2026-09-07-plated-terminal-via-cleanup.md)
found that via-removal actions unnecessarily drilled a via at a plated
through-hole terminal. Correcting that endpoint handling lets the existing
search select an improvement on `SIG03_R_HEADER`: the completed open board
drops from 18 to 17 vias with unchanged track length and all 19 connections
natively valid. This is a retained post-routing result; the baseline comparison
above remains the original routing output. General automatic board-wide
cleanup and the remaining via choices are still under investigation.

The [initial router now considers plated-terminal layer choices](../../docs/reviews/2026-09-07-plated-terminal-layer-search.md)
as well. Replaying `SIG03_R_HEADER` on the identical full source produces one
via directly, with 42,646 expansions instead of 75,399; all 19 connections
remain natively valid. This reproduces the earlier cleanup improvement during
routing and does not change the historical profile results above.

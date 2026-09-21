# Fresh whole-board evaluation

The frozen original-placement evaluation is complete. Both history and boundary
priority route three of the five pinned sources; neither finishes `interf_u`
within 30 minutes, and Olimex fails source preflight. This does not establish a
whole-board completion benefit from boundary priority or deeper recovery.

Every run starts with zero copper and retains the source placement, netlist,
physical rules and outline. The executable, configurations and per-job limits
were frozen before execution. Four physical cores carried independent streams;
each timed job used one core's two hardware threads. Resumed checkpoints do not
count toward these results.

| Source | History priority | Boundary priority | Native qualification |
| --- | --- | --- | --- |
| ECC83 | 9/9 nets | 9/9 nets | Two unchanged source silkscreen warnings |
| PIC | 34/34 nets | 34/34 nets | Zero native findings |
| Complex hierarchy | 50/50 nets | 50/50 nets | Zero native findings |
| Interf-U | Timeout: 51/110 nets, 147 opens | Timeout: 48/110 nets, 150 opens | Retained partial boards have zero physical/ERC/parity findings |
| Olimex ESP32-C3 | Source preflight failure | Source preflight failure | Six inherited fiducial-to-hole clearance errors, plus library/annotation/parity findings |

All three completed pairs produce identical copper within each pair and finish
in their first pass. Complex hierarchy uses only the coarse grid, with no
repair. Its third, demand-enabled arm also stops on that same first control
pass. These are regression controls; they never exercise demand or learned
ordering. The Interf-U arms do not finish their first adaptive pass. The
Olimex result remains in coverage accounting; native net-class compilation
alone had not established source health.

History jobs take about 135 seconds for ECC83, 704 for PIC and 681 for complex
hierarchy. Single-sample timing differences under concurrent load are not
attributed to priority changes. Timeout CPU/RSS observations omit unreaped
descendants and are reported unavailable, not as low utilization.

[Frozen protocol](../../build/whole-board-reset-2026-09-08/evaluation.json),
[ECC83/Interf results](../../build/whole-board-reset-2026-09-08/fixed-placement/completed-results.json),
[PIC evidence](../../build/whole-board-reset-2026-09-08/restrictions-runtime/findings.md),
[complex results and combined renders](../../build/whole-board-reset-2026-09-08/global-routing/complex-hierarchy/index.html),
[Olimex findings](../../build/whole-board-reset-2026-09-08/global-routing/olimex/results.json).

## Changes justified by the measurements

Independent forecasts were unused by successful first passes but consumed
214 seconds on PIC and 154 seconds on complex hierarchy. Deferred forecasting
now passes completed-board, four-pass learned-order continuation and explicit
optimization controls. ECC83 and PIC retain byte-identical final boards. A
separate fresh Interf-U candidate reaches 55 nets/143 opens before the same
30-minute cap, preserving an admitted first pass before timing out during
forecasts. Its first 51 committed boards match the eager baseline exactly.
This is useful progress within the budget, not routing completion.
[Candidate result and rendering](../../build/whole-board-reset-2026-09-08/lazy-fresh/result.json).

Two successful Interf-U repairs consumed 425 seconds. One evaluated three more
admissible alternatives after the first success for only 2.56 mm improvement
in its selection score. An opt-in first-admitted repair policy is being tested;
its fresh Interf-U evaluation retains 57/110 nets and 140 opens before the same
30-minute cap, timing out during forecasts. It does not complete the board.
Its changed early choice makes a later repair more expensive, and total recorded
step time is 1,449 seconds versus 1,252 for the pure-lazy control. It remains
experimental; a local stopping rule is not a uniform runtime improvement.

A matched opt-in retry of observed failures before forecasting does not improve
Interf-U's incumbent: both arms time out at 55 nets/143 opens. Their first-pass
copper, selections, work and native findings are identical. The enabled arm
spends 440.6 seconds on a seven-net retry before failing at PC-A4, then starts
forecasts; the original pass remains selected. This policy stays experimental.
The off arm was repeated unchanged after an unexplained outer-process
interruption, which is recorded separately from solver failure. Shared-load
timings are descriptive.
[Matched result](../../build/whole-board-reset-2026-09-08/global-routing/observed-retry/matched-result.json).

Placement is evaluated separately. The coupled legalizer converges faster to
essentially the same complex-hierarchy placement, and both placements route
all 50 nets. Mechanically anchored ECC83 instead fails; its initial harmonic
seed overlaps fixed occupied area. Bounded contact-direction search does not
solve it. Obstacle-aware initialization was then tested with a second anchored
PIC case frozen before implementation. All four off/on placement arms fail:
ECC83 initially clears fixed-space collisions but legalization reintroduces
them; PIC exhausts the same pair-check budget with either setting. No arm earns
a cold route. These mechanisms remain experimental. Neither these placement
experiments nor the supplemental generated SMD cases replace any source in this
table.
[Placement results and animations](../../build/whole-board-reset-2026-09-08/placement/fixed-obstacle-seed/results.json).

The next single-order insertion experiment yields a legal anchored ECC83
placement in 0.446 seconds; its fresh cold route connects all nine nets in pass
zero (121.8 seconds). Anchors and outline remain exact. Its 43 annotation
findings initially prevent full native admission; physical/ERC/parity checks pass.
One separately timed invocation of the existing silkscreen repair clears all 43
in 17.6 seconds and earns full native admission, with three library metadata
warnings. Exact copper, component poses and outline remain unchanged.
[Final native result](../../build/whole-board-reset-2026-09-08/placement/sequential-seed-insertion/ecc83-annotation-postprocess/result/verification.json).
Anchored PIC exhausts the unchanged 1M pair-check budget during insertion at R9.
No additional order or increased budget has been tried.
[Insertion results](../../build/whole-board-reset-2026-09-08/placement/sequential-seed-insertion/results.json).

A subsequent geometry optimization rejects provably separated footprint pairs
before exact collision checks. All 59 placement tests pass; ECC83 preserves
exact placement, copper and routing work. At the unchanged 1M exact-check cap,
PIC now inserts 39 of 52 free components before failing at C7; its first 19
poses exactly match the previous diagnostic. Nearly all 946,851 candidate
positions visited collide with occupied space. The unadmitted prefix is rendered
for diagnosis, and production retains atomic rollback. This is a search-work
improvement, not a completed PIC placement or area result.
[Comparison and renders](../../build/whole-board-reset-2026-09-08/placement/seed-projection-broadphase/index.html).

Skipping certified colliding intervals subsequently completes PIC's 52 free
placements in 0.192 seconds of search at the unchanged 1M cap. Interval
preparation, row certificates and exact checks consume 362,259 operations;
the final verification sweep brings the total to 364,212. All 11 anchors,
five outline edges and the prior 39 poses remain exact. Native geometry,
ERC and parity pass, with 125 cold opens and 52 annotation findings still
present. ECC83 retains its complete previous placement; 65 placement tests
and four prior-result controls pass. Cold routing is being evaluated separately.
This establishes placement capability, not a completed or smaller board.
[Placement and native evidence](../../build/whole-board-reset-2026-09-08/placement/collision-intervals/results.json),
[PIC rendering](../../build/whole-board-reset-2026-09-08/placement/collision-intervals/pic-insert/area-00/trace/final.svg).

The supplemental SMD pair exposed an endpoint-trimming correctness bug: trimming
an in-pad path prefix could delete a necessary layer transition. Preserving it
passes 145 KiCad tests, the motivating native control at unchanged search work,
and an unchanged fresh ECC83 regression. The fresh blocked-pad-gap case now
finishes all 19 selected connections in pass zero (329.5 seconds), with zero
opens/physical/ERC/parity findings and 21 unchanged metadata warnings. The
original executable exhausts four passes at 11/19 and 12 opens (1,551.7 seconds).
Both use the same frozen placement, rules, policy and maximum budget; this is a
completion gain on a generated supplemental case. The corrected open-pad-gap
case also completes all 19 connections in pass zero (220.8 seconds), with zero
opens/physical/ERC/parity findings and the same 21 metadata warnings. Its
original control exhausts four passes at 9/19 and 14 opens (977.3 seconds).
Both corrected cases preserve all eight rule
areas and the common placement. Against the matched Freerouting references,
both use less track but more vias; this does not establish superior quality.
[Corrected result](../../build/whole-board-reset-2026-09-08/restrictions-runtime/supplemental-smd-trim-corrected/blocked/run/run.json),
[combined copper](../../build/whole-board-reset-2026-09-08/restrictions-runtime/supplemental-smd-trim-corrected/blocked/run/adaptive/result/preview.svg).

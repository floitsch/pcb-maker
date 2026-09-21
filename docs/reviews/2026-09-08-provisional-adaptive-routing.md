# Adaptive routing on generated native placements

The general placement pipeline can now use learned net order and future-net
demand while source annotation cleanup remains unfinished. This is provisional
routing progress: a board with any remaining native design finding still fails
the complete-layout gate and earns no area credit. Local via optimization
remains [deferred](../optimization-todos.md).

## Admission and integration

`route-kicad-board-adaptive` accepts an explicit
`allow_existing_annotation_findings` configuration flag, defaulting to false.
Only `silk_overlap`, `silk_over_copper` and `silk_edge_clearance` findings may
remain provisionally. Each current finding must occur in the source findings,
including its affected object UUIDs, descriptions, severity, positions and
multiplicity. Item ordering is normalized. A new finding cannot replace an old
one merely because their types or counts agree. ERC and schematic parity must
remain clean; all physical design findings still prevent admission.

Result schema 2 distinguishes `routing_complete` (no remaining native open
items) from `complete` (the full native gate). It reports
`outstanding_annotation_findings`; every pass records
`native_progress_admissible`. A complete incumbent outranks a provisional one,
regardless of route length. Otherwise fewer opens precede the existing routing
score. Native result verification is repeated after copying the selected board.
The CLI exits successfully only for a fully complete layout.

`area_probe.py --route-with-annotation-findings` now passes the explicit opt-in
to the adaptive coordinator, replacing the earlier sequential fallback. Its
report distinguishes connected copper with outstanding findings from an
admitted layout. The independent report auditor recomputes the finding subset
and incumbent selection from raw native DRC evidence.

All 121 KiCad unit tests pass. The new regression checks default rejection,
explicit provisional acceptance, replacement findings on different UUIDs,
physical/ERC/parity rejection and complete-incumbent protection. The native
strict-mode control rejects the annotated complex source; clean ECC83 still
completes in one pass with zero ERC/design DRC/parity/open items.

## Experiment scope

The larger experiment uses the previously generated 68-component, 165-pad,
50-net `complex_hierarchy` placement, at its source area of 8057.413 mm².
It starts with zero copper, 112 native open items, seven annotation errors
and eight separately recorded library metadata warnings. All four routing
passes use the same placement and source routing dimensions. The grid is
0.5 mm, with a one-million-expansion search bound and at most eight shared-tree
attachment searches. One independent forecast is generated per net.

Every native verification automatically captures labelled combined front/back
copper with component bodies hidden. Forecast previews are explicitly
unverified geometry, not native completion evidence. All fifty independent
forecasts find paths; that does not establish simultaneous physical feasibility.

Executable, source snapshots, configuration, test/build logs, process outcomes
and native evidence are under
[`build/adaptive-complex-2026-09-08`](../../build/adaptive-complex-2026-09-08).
The comparison report is generated only after the commands have terminated.

The end-to-end driver control also exits normally. Its extracted problem and
placed result exactly reproduce the preceding complex-board run; its one-pass
adaptive invocation retains 58 opens and seven annotations. The driver reports
`native routing incomplete`, records the routing command's exit code 1 and
leaves the best admitted area unset. This validates the new pipeline path
without awarding completion for a successfully executed but incomplete run.

The independent audit passes for the four-pass experiment, driver control and
clean ECC83. It checks native poses and requested copper dimensions, reproduces
incumbent selection, checks annotation identity, and validates 254 SVG files.
Player frame paths and JavaScript syntax pass; browser playback was not tested.
See [combined comparisons](../../build/adaptive-complex-2026-09-08/index.html)
and [terminal process outcomes](../../build/adaptive-complex-2026-09-08/processes.json).

## Four-pass result

| Pass | Policy | Routed nets | Native open items | Time, seconds |
|---|---|---:|---:|---:|
| 0 | Original configured order and attachment objective | 13 | 58 | 86.3 |
| 1 | Same order, router-cost attachment control | 14 | 56 | 74.8 |
| 2 | Same order, future-net demand strength 1 | 23 | 42 | 112.1 |
| 3 | Learned order, router-cost attachment control | 30 | 65 | 188.6 |

Pass 2 is retained. All four have the same seven source annotations and no new
native findings; none completes. Counting fully routed nets alone would favor
pass 3 incorrectly: its remaining multi-terminal connections leave more native
open items. Independent forecasting took 43.2 seconds. These are individual
recorded runs, including native verification, not general performance claims.
The complex-board benchmark configuration now enables this four-pass budget
and demand strength for future runs.

The demand pass next fails at `Net-(U301A--)`: the reachability preflight
exhausts its reachable region after 337 expansions. Its independent empty-board
forecast finds a path. This is a useful target for identifying the particular
earlier copper that blocks terminal escape, then selectively rerouting it.
It is not evidence that increasing the A* expansion bound would solve this
failure. The present coordinator restarts whole passes; it does not yet retain
successful route prefixes or negotiate specific blocking nets.

The next completion work should connect that conflict evidence to local
rip-up and, where necessary, component movement. Adapter coverage for more
courtyard shapes and outline constraints remains a separate breadth task.
Neither remaining silkscreen cleanup nor these routing improvements may be
counted as finished until the full native board verifies cleanly.

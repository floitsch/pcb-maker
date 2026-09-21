# Stop routing once connectivity is complete

The adaptive coordinator now defaults to stopping after the first accepted
native result with zero open connections. Further copper/via optimization
requires `optimize_after_routing_complete: true`. This makes the user's
whole-board completion priority the program's default behavior.

The immediate evidence was the spectral `complex_hierarchy` run: pass 0
connected all 50 nets, but pass 1 spent another 1631.6 seconds rerouting the
same fixed placement without improving the retained result. Three fixed
silkscreen findings remained throughout. That is measured redundant work in
the archived run, not an isolated speed benchmark for the new executable.

## Native controls

[Combined copper views, playback and independent audits](../../build/completion-first-2026-09-08/controls/index.html) ·
[Control receipts](../../build/completion-first-2026-09-08/controls/controls.json) ·
[Incomplete-budget control](../../build/completion-first-2026-09-08/budget/index.html).

| Control | Finished passes | Native opens | Design findings | CLI exit |
| --- | ---: | ---: | ---: | ---: |
| Default completion, ECC83 | 1 | 0 | 0 | 0 |
| Explicit optimization, two-pass budget | 2 | 0 | 0 | 0 |
| Default completion with deliberate silkscreen conflicts | 1 | 0 | 6 | 1 |
| Exhausted routing budget | 2 | 20 | 0 | 1 |

All three connected controls reproduce the archived first pass's copper
geometry exactly. The annotation control retains its source findings and
does not earn complete-layout admission. The incomplete control explores both
distinct queued policies and retains the original board byte for byte; it
does not terminate early merely because a pass finished.

Independent audits reproduce native admission, fixed input/pose preservation,
requested copper dimensions, incumbent selection and automatic rendering
references. All 130 KiCad tests and eight benchmark tests pass. The frozen
executable and source snapshot are retained under
`build/completion-first-2026-09-08`.

## Contract

The new field defaults to false when absent. Existing configurations therefore
adopt completion-first behavior; replaying a historical multi-pass quality
experiment requires setting it to true. `maximum_passes` remains the upper
bound on incomplete search and opt-in optimization.

Result schema 3 reports early termination as `routing_complete` and keeps
`remaining_queued_trials` visible. `routing_complete`, full native `complete`,
and outstanding annotation findings remain separate. The final retained board
still receives fresh native verification before the command returns.

The already-running placement comparison and full 110-net `interf_u` run use
their frozen earlier executable and retain their original budgets. They are
not restarted or reinterpreted as tests of the new default.

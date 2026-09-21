# Distinguishing an expensive search from a disconnected grid

The `interf_u` run stopped at 55 of 110 nets, with 143 native open items. Its
next net, `OE-`, was disconnected on the 0.5 mm grid but reachable on the
0.125 mm grid. Ordinary A* exhausted four million expansions without finding
the latter path. The whole-board coordinator treated both failures as strings
and recorded zero work for each failed attempt.

An equal-input probe with the existing obstacle-distance heuristic finds a
native-admissible route for `OE-`. It uses 163,052 A* expansions and 1,063,655
reverse-distance preparation expansions. The failed control uses 4,000,001
A* expansions plus 161,332 reachability expansions. These are work counts,
not a matched wall-clock speed measurement. Both use the same grid, costs,
dimensions, retained copper and A* bound.

[Combined copper comparisons and retained progress](../../build/interf-budget-guidance-2026-09-08/index.html).

## Production change

The sequential coordinator now retains the router's typed search failure:
budget exhaustion or grid disconnection, the failed branch, and separate
A*, reachability and heuristic-preparation work. It does not infer these
categories from error-message text. Known failed-search A* work now contributes
to the attempt and total counters; historical counts are not reconstructed.

`obstacle_distance_fallbacks` maps an explicit guided portfolio index to an
earlier geometric index. For example, `{"2": 1}` makes entry 2 conditional on
entry 1 exhausting its search budget. Validation requires that the entries
differ only in their heuristic, including identical geometry, source rules,
costs and A* budget. The retry cannot quietly relax rules or expand the budget.

Both ordered-first-admitted and best-score policies support the trigger.
Skipped entries receive explicit zero-work records, preserving portfolio
indexing and native selection semantics. Disconnection, unrelated failures,
and ordinary candidate generation do not activate the retry. All generated
candidates still require native admission before any commit.

The native placement-routing driver appends these conditional entries by
default, after the ordinary portfolio. `--no-budget-guidance` preserves the
supplied portfolio without adding entries. Explicit guided entries are retained
with their declared policy. Direct sequential configuration remains opt-in.
Independent adaptive forecasts exclude these conditional retries: forecasts
have no retained-copper search failure to trigger them.

Journals use schema 7. Legacy schema 3–6 checkpoints remain readable, and new
schema 7 checkpoints can themselves be resumed. Optional typed-failure and
skip fields default to absent when reading old records.

## Native recovery and controls

- Resuming the verified 55-net prefix automatically routes `OE-` through the
  new conditional entry, reaches 56 nets / 142 opens, and stops at the deliberately
  selected bound. The predecessor board and all earlier commits are retained.
- Resuming that schema-7 checkpoint then routes `PC-A0` with ordinary coarse
  guidance, reaching **57 nets / 140 opens**. `PC-A1` fails on branch 1 with
  disconnection in both grids; its guided retry is explicitly skipped.
- The separate 52-net `MD5` checkpoint is disconnected at both resolutions.
  Its guided retry is also skipped, with no new copper or findings.
- All three independent sequence audits pass: retained prefix, unchanged poses
  and unrelated copper, source projects, native dimensions, and connectivity
  accounting. Their new-step audits also reproduce the conditional decisions.
- All 137 KiCad adapter tests and eight benchmark tests pass. Added controls
  reject changed-rule retries, invalid indices, unrelated or opaque failures,
  and invented evidence in old records. Independent audit mutation controls
  reject an untriggered retry, a skipped required retry, and lost failed work.
- Every native attempt retains automatic combined front/back copper renders
  without component bodies. Final retained rendering was visually inspected;
  this study does not claim a new browser playback test.

These results remain incomplete boards. No whole-board completion, reduced
area, physical impossibility, or equal-rule external-router superiority is
claimed. The next useful diagnostic is which retained copper separates the
failed `PC-A1` branch, not a larger A* budget on the same disconnected grid.

Artifacts and frozen binaries are under `build/interf-budget-guidance-2026-09-08/`.

## Unchanged-board and forecast control

On the final executable, ECC83 completes all nine nets in its first pass. Every
ordinary candidate exactly matches the archived completion-first control, as
does the final copper hash. The best-score policy records all nine conditional
entries as skipped. Independent forecasts make two attempts per net despite
a limit raised to four, proving that the new conditional entries are excluded.
The native sequence and adaptive-selection audits both pass.

The first recovery executable is recorded separately from the final executable
that additionally excludes conditional entries from independent forecasts.
`binary.json` and both source snapshots preserve that distinction. The
sequential recovery implementation is unchanged between them.

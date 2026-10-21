# Review: benchmark harness and the claims made from it (2026-10-21)

Scope: the agent-tasks harness and its tests, benchmarks/expansion,
check.sh, prune.sh, CI, README, benchmarks.md (v18, designer tier),
agents.md's pass rule; commits 98efa4a, 36f24ba, 043c247, 9c7abf3. Nothing
built or run but the Python unit tests (37 passed) and compare.py over the
stored logs. "Sure" means confirmed in code or in
logs. "Suspect" means the mechanism is real but its effect is not measured.

## High

**H1. A killed route run produces a row that looks like a result (sure, seen in logs).**
In route mode the ladder moves each new best attempt into the output
directory while the run is still going (crates/pcb-kicad/src/board_router/ladder.rs:1868-1872,
2279, 2584). `route_result` (run.py:1276-1312) reads `route/board-router.json`
and `drc.json` without checking the exit code. If the process is killed by
OOM or a signal, the row therefore gets `routed`, `native` and `pass` with
no `error` and no `cut`. compare.py then counts it in better/same/worse.
`route_task_admitted` makes it worse: if the killed route left a board but
no drc.json, it runs drc-kicad-board on that partial board (run.py:1600-1608).
Three such rows exist: build/fr2-route-x1367.log (bumwings_v001_core and
ghoul, both `exit: -9`) and build/fr2-route-x1371-r2.log (katia, `exit: -9`,
93 open). All have `verdict: null`, `error: null` and `cut: null`. Layout
mode is safe, because board-layout.json is written last
(board_layout.rs:2023). Even there, a killed layout's `error` is the last
line of the output, not "killed by signal 9". Fix: a negative code, or a
code outside 0..4, or a report without a verdict should set `error` and
`cut: ["killed"]`. Also skip the extra DRC in that case.

**H2. A finding with one item matches on uuid alone, so its position is ignored (sure).**
In `same_place` (run.py:298-321, the same in verdict.rs:293-330), the check
that items stayed the same distance apart has nothing to compare when there
is only one item. Any finding of the same type on the same uuid then
counts as the designer's. KiCad reports a zone item at a position that does
not depend on where the problem is (seen: (0, 0)). So a `connection_width`
finding on a pour that our tracks cause matches any designer
`connection_width` on that pour. This happens in the headline runs: the
designer-tier strict pass of USB_Keypad (designer-x1315b.log) subtracts one
`connection_width` on zone 8951654c at (0, 0), and the stripped baseline has
45 findings there. The layout row of the same board subtracts 6 of them. If
that finding is ours, the designer tier is 49 rather than 50, and the
"placement" group has 12 boards, not 13 (suspect: KiCad's report gives no
location to tell). docs/agents.md:400-405 and the tasks README describe "the
same items, the same distances apart" and do not mention this case. Fix:
for one-item findings on a zone, require more (the description's "actual"
value, or the violation's marker position if KiCad writes one), or count
them as new.

**H3. The 71/29 placement/routing split mixes placement with routing budget (sure on mechanism, size unmeasured).**
The designer tier gives the router the full 1500 s of work as
`ladder_budget_seconds` (run.py:1318, 1357-1360). In the layout, the
placement race stops at 40 % of `total_seconds`, the moves at 65 %, and the
final ladder gets what is left (board_layout.rs:40-45). On the hard boards
the split is about, the "routing gap" was therefore measured with roughly 3
times the routing work our layout's router gets. Whatever is lost to that
smaller budget shows up as "placement gap". "The two runs of a board get the
same work" (run.py:1318 comment) is true of the total only. In addition,
splitting by subtraction assumes the two effects add up, so the README's
"our placement accounts for 71 %" is an accounting split, not a cause. The
numbers themselves reproduce (see below). Fix: say this next to the split,
or run the designer tier at the layout's final-ladder share as a bound.

## Medium

**M1. Rule 2 subtracts without limit and also uses the designer's own routing (sure, documented, not bounded).**
`prebridged` (run.py:273-295) has no one-for-one bookkeeping: one bridged
opening absorbs every bridge of ours on it. It also treats an opening as
"pre-bridged" when it bridges only on the designer's own board *with its
tracks* (StrictBaseline.designer, run.py:422-431). That bridge comes from
the designer's routing, not from the fixed geometry, so our tracks crossing
that opening anywhere are exempt. In layout mode the exemption moves with
the part, because it matches on uuid. Today this affects only Sisu (14/17),
with no verdict change. The rule as described ("an opening that bridges on
the designer's board") matches the code. "Only the designer's findings at
the same place are subtracted" (README, compare.py:30) no longer holds
literally.

**M2. Rows do not record the binary or where a spliced row came from (sure).**
`spliced` is a bare `true` (run.py:1679-1683). No row, summary.json or .err
file names the binary. That v18 a/b/c and the designer tier are "the same
binary x1315" is therefore stated only in the docs. github-layout-v18.log
says "105 rows spliced from build/github-layout-v18b.log", but 41 of those
rows came from v18a (6 x 2, 20 GB kept free). The summary.json of a spliced
run describes only its last part: v18c reports `jobs: 2`, `wall_seconds:
5211` and a measured-rate median of 1.78e6 over 4 rows. The docs quote
2.29e6 over all 109 rows, computed by hand (`measured_rates(ran)`,
run.py:1911). Fix: put `binary`, its sha256 and `spliced_from` in each row.

**M3. Rule labels on spliced runs, and rule 2's re-judging is not logged (sure).**
None of the five headline logs carries `strict_rule`, so they are rule 1.
The logs have strict 1550 and 1256. The rule-2 figures (1536 and 1239, both
42/50) come from a scratch script (changelog:709) and are not in any named
log. The counts quoted are the same under both rules, so the headline
stands. However, run.py now prints "strict rule 2: N pass" and writes
summary.json `strict_rule: 2` even when the spliced rows were judged under
rule 1 (run.py:1904-1911). compare.py handles this correctly
(`rules_warning`).

**M4. A timeout leaves kicad-cli children running while the memory gate admits the next board (sure).**
`run_measured` kills only the direct child (run.py:733-736; the comment at
742 acknowledges the survivor). `subprocess.run(timeout=...)` in
designer_findings and StrictBaseline behaves the same. The gate releases
the board's reservation (run.py:846-849) while an orphaned kicad-cli
(2.2 GB or more, up to 900 s) is still running, and that orphan's CPU is
never counted. On this memory-bound machine, that is exactly how a
following route can be OOM-killed into H1. Fix: start the child in its own
process group and kill the group.

**M5. Timeouts after the stretch cap (sure).**
With rates pinned, a layout's timeout is 1500 x 2 x 4 + 300 = 12300 s
(run.py:880-896), up from 3300 s. A hung binary holds a slot and its memory
reservation for 3.4 h. The factor is always 4 (`WALL_STRETCH_MAX`), whereas
the binary stretches only by pinned rate over measured rate, and honours
`PCB_WALL_STRETCH_MAX` (work_rate.rs:356-363). Above 4, the harness cuts
the run before the binary's own guard would. Those rows are marked cut, so
this errs on the safe side.

**M6. What check.sh and CI do and do not gate (sure, except one suspect point).**
Gated: the three core crates' tests, the CLI tests including
`config_reference` (the staleness test runs in CI through check.sh), all
three Python test directories including benchmarks/expansion, rustfmt
`--all`, and a release build. Not gated:
- `cargo test -p pcb-maker --test ...` (check.sh:42) skips the binary's own
  unit tests: 4 in src/freerouting_backend.rs and 2 in src/agent_cli.rs.
- The unit tests of tools/config-reference (4), pcb-congestion (5) and
  pcb-topo (8) never run (check.sh:41). pcb-kicad depends on the last two.
- clippy runs on `-p pcb-router -p pcb-kicad -p pcb-placer` only
  (check.yml:62). src/main.rs (87 lines changed today) and
  tools/config-reference are never linted. Whether congestion and topo get
  linted as path dependencies is suspect.
- GitHub runners have no kicad-cli, so every KiCad-dependent test reports a
  skip and passes. The fingerprint test checks only that the result does
  not depend on the thread count; it compares against no golden value.
- With `--no-walkthrough`, `"${walkthrough[@]}"` is an empty array under
  `set -u`. That fails on bash older than 4.4 (macOS /bin/bash). Suspect,
  matters only for macOS.

## Low

- **L1** (sure): compare.py prints "strict - -> 42 of 103 rows" for v17 vs
  v18. Error rows have no `pass_strict` and are treated as written before
  the rule (compare.py:250-257, 702-707). They should count as failures.
- **L2** (sure): rows added with `compare.py --splice` are not marked SPL
  and are not in the "rows spliced" line (compare.py:630-633).
- **L3** (suspect): compare.py works out passes again without the schematic
  tier's `missing` condition (run.py:1195-1196 vs compare.py:241-257). A
  schematic row with missing footprints can count as a pass in compare.
- **L4** (suspect): the position fallback is greedy. One of our tracks that
  starts at a fixed pad P's centre can take the designer's P-Q finding. The
  count usually stays the same, because our own P-Q finding then counts as
  new, but the assignment depends on order.
- **L5**: the README table puts x738's "62 clean" (per-type rule, every
  copper zone removed, wall budget, a binary 577 snapshots older) in the
  same column as the designer tier's 50 under the strict rule.
  benchmarks.md:1360 explains the difference; the README does not.
- **L6**: kicad-cli timeouts inside the binary (0xCB pcb-panel, "timed out
  after 900 s") depend on the wall clock but are not marked `cut`.

## Question 3: splicing

run.py marks spliced rows, and compare.py marks them SPL, counts them in
the pass totals and reports how many there are ("after: 105 of the 109 rows
spliced"). The pass counts are right: 46 and 42 in both run.py's line and
compare.py's. The cut reruns (katia, Batch_1) are rows that ran rather than
spliced ones, and they are left out of better/same/worse as cut. The
problems are the lost source (M2) and the rule label (M3). The v18a stop
left no rows from killed boards: its exit codes are 0, 1 and 4 only.

## Question 2: what reproduces

Reproduced from the named logs: 46/42 of 109 (github-layout-v18.log,
byte-identical to v18c.log); 58/50 (designer-x1315b.log); the medians
x0.71, x1.03, 0.009 and +0 mm; the 18/24/6 split of the 67 failures;
`compare.py --gap` (101 boards, 2222/1153, 48/52 %; without the four
non-targets, applied by hand, 97 boards, 1483/430, 71/29 %); 13/5/37/44
plus one board without a result; v17 against v18 (5/87/7, 3 cut); task
list sha c8f95caf in every row; Freerouting x738 (67/36/6, 47/6, 51/8,
CPU 40885/62688 = 0.65) against github-freerouting-x738-compare.txt. Not
traceable: the rule-2 re-judging (M3), the binary's identity and the
measured-rate medians (M2).

## Question 1: is the strict rule as documented?

Yes: matching by uuid or by position within 0.05 mm, the cap of 199 checked
against the stripped baseline, and rule 2's test for an opening (by
description: Track, Via or Zone excluded; a pad or something on F.Mask or
B.Mask counts) with a track or zone as the other item. The Python
`same_place` matches verdict.rs. A real new error of ours can still be
subtracted in three ways: a one-item zone finding (H2, seen), the open-ended
and designer-routed openings of rule 2 (M1), and, rarely, the greedy
position fallback (L4). Our vias and other parts' pads under an opening do
count, as the tests show (test_run.py:192-206).

## Question 6: dead and duplicated code

An AST scan finds no unused top-level function in run.py, compare.py,
quality.py or benchmarks/expansion. Duplicates:
- The DRC counting filter appears three times in run.py
  (`counting_findings`, `native`, `designer_findings` at 241/516/568).
- The copytree ignore list appears three times.
- strip-kicad-tracks is called inline (run.py:919, 408) beside
  `strip_command`.
- board-router.json is read again after `route_result` in two runners.
- `file_sha256` (run.py, quality.py); `atomic_json`, `digest`,
  `read_json`, `safe_name` across harvest, triage and import_pending.
- There are three memory gates (run.MemoryGate, triage's gates,
  import_pending.available_memory).
- `intake_outcome` re-implements triage.classify, so the two can drift.
- compare.py's `passes` re-implements run.py's pass rule (L3).
- Rule 2 exists only in Python; the binary's verdict stays on rule 1, as
  documented.

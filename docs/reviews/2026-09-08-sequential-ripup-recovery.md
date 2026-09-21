# Automatic recovery during whole-board routing

The sequential router can now invoke the tested partial-board rip-up action
after ordinary routing fails, commit a restored board, and continue with the
next net. This connects layout diagnosis to the main routing loop. It retains
the successful routing prefix instead of restarting every net after a blockage.

## Behavior and bounds

The optional `ripup` configuration supplies `maximum_invocations`,
`maximum_diagnosis_trials`, a `routing_config_index`, and the annotation policy.
Defaults for an enabled repair configuration are four invocations, 64 diagnosed
single-net removals per invocation and portfolio member zero. Omitting `ripup`
preserves the previous behavior. The adaptive coordinator passes its explicit
annotation policy to the repair configuration.

Recovery runs only when no ordinary portfolio candidate was admitted. It uses
the selected portfolio member with forecasts for completed nets removed. The
partial-repair command diagnoses yielding nets, inserts the target, restores
each candidate yielded net, and retains its best admissible complete repair.
The outer sequential coordinator then verifies the candidate again against
the original source ERC inputs, DRC findings and exact connectivity ledger.
Only that admitted board replaces the retained result. Failed and unselected
repair artifacts remain separate from the retained board.

Each committed target advances the configured connection order once, whether
ordinary routing or recovery supplied it. Every prior net remains connected.
Repair invocations have a separate bound from attempted connections, and a
later failure explicitly reports when that repair budget is exhausted.
No recovery is attempted before any earlier copper has been routed.

Sequential report schema 4 has separate repair receipts; an ordinary failed
candidate is never relabelled as selected. A receipt records the repair
directory, yielded net, selected attempt, resulting target/yielded qualities,
native admission and route expansions. Repaired steps appear in adaptive
playback. Adaptive difficulty learning uses the latest geometry of a net that
was rerouted by a later repair, and recognizes successfully repaired targets
as routed. The order-search connectivity ledger also counts repaired steps.

## Verification scope

All 124 KiCad unit tests pass. Tests cover finite repair bounds, valid portfolio
selection, learning from repaired targets and changed earlier nets, as well as
the preceding partial-repair admission checks. Native experiments use the same
68-component/50-net generated complex-board placement and source dimensions.
They compare the retained demand policy with a router-cost control, both with
four allowed repairs. A clean ECC83 run checks enabled-but-unused recovery.

The independent recovery auditor reconstructs the connectivity ledger from
native evidence, verifies every retained predecessor, and checks unrelated
copper and component poses through isolated KiCad readback processes. It also
checks repaired-net dimensions, invocation bounds, unchanged source inputs and
the unchanged ordinary prefix against the preceding demand run. Native
verification automatically renders combined front/back copper; the audit
assembles a step viewer that includes repairs.

Artifacts and process outcomes are under
[`build/sequential-recovery-2026-09-08`](../../build/sequential-recovery-2026-09-08).
Routing checkpoints do not establish process termination or full native layout
completion. Final native findings remain separate from routing progress.

## Recorded outcome

The demand-guided command exits normally with an explicit incomplete-routing
exit code. It commits 27/50 nets, leaving 36 native open items and the same seven
annotation findings. Its first 23 committed boards are byte-identical to the
previous demand run. Recovery inserts `Net-(U301A--)` by yielding
`Net-(D304-K)`, then later inserts `Net-(R312-Pad2)` by yielding
`Net-(U102-CAP-)`. Ordinary insertion resumes after the first repair.

The third recovery, for `Net-(C201-Pad2)`, finds only one useful single-net
removal among all 27 earlier nets: `+12V`. Restoring `+12V` then fails, so that
temporary removal is rejected and the preceding 27-net board remains intact.
This is not repair-budget exhaustion: three of four invocations were used.

Two frozen-parent probes refine the grid from 0.5 to 0.25 mm, preserving physical
trace/via dimensions, clearances and the physical bend/via cost tradeoff. Direct
`Net-(C201-Pad2)` insertion still reports grid disconnection, as does `+12V`
restoration after temporary target insertion. The latter reaches branch 3;
its aggregate expansion count includes earlier branches. Neither failed probe
proves physical infeasibility or rules out different tree attachment choices.
The completed run's independent audit validates all 27 committed steps, both
repairs, the unchanged 23-step prefix and 71 SVG files. The interrupted
checkpoint's 25 committed steps also pass the native readback audit; its
uncommitted alternatives are not counted. See the
[combined report and step viewers](../../build/sequential-recovery-2026-09-08/index.html).

The clean ECC83 adaptive control finishes with zero native design findings or
opens, never invokes recovery and reproduces the earlier board byte for byte.
Its nine-step native audit passes.

The no-demand adaptive comparison is **interrupted**, not completed. Its session
handle disappeared across execution-session turnover, no router or KiCad CLI
process remained, and no final adaptive report exists. Its durable checkpoint
has 25 routed nets / 39 opens and one committed repair. The in-flight `VCC`
repair has eleven of thirteen restoration attempts recorded, but no selected
repair result or sequential commit. These artifacts remain diagnostic evidence;
they do not establish a finished policy comparison.

The complex-board benchmark configuration now enables bounded recovery.
[Verified checkpoint resumption](2026-09-08-sequential-checkpoint-resume.md)
subsequently retained the interrupted 25-net prefix and advanced to 26 nets /
37 opens in a bounded continuation. The remaining restoration conflict should
now drive broader repair/placement choices.

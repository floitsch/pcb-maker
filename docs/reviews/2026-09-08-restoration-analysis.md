# Restoring a displaced net and exposing the repair chain

The previous `interf_u` result stopped at 62 nets / 131 opens. Yielding
`PC-A3` let `PC-A4` route, but the displaced `PC-A3` could not be restored.
The coordinator correctly rejected that provisional board.

The existing bounded nested-repair mechanism resolves this case with one
additional repair level. After placing `PC-A4`, it restores `PC-A3` by yielding
`PC-A2`, then restores `PC-A2`. The parent admission checks the whole change,
including all three nets, protected `PC-A4`, unchanged unrelated copper/poses,
source rules and expected native connectivity. It accepts **63 nets / 129
opens**, with no native design/ERC/parity findings.

[Compact repair-chain report](../../build/interf-restoration-2026-09-08/depth-one/analysis.html).

## Continued routing

Resuming that checkpoint with four outer repair invocations, eight outer
diagnostic trials and the same one-invocation/one-level/eight-trial nested
budget reaches **65 of 110 nets / 125 opens**, with no native design findings.
`PC-A5` uses a nested repair whose accepted changes are `PC-A0`, `PC-A4` and
`PC-A5`. `PC-A6` then commits a direct repair of `PC-A5`.

`PC-A7` remains unresolved: none of the eight tested yielding candidates
produces a route. This is a truncated diagnosis over the recent history, not
a proof of physical infeasibility. An older blocker, a different target path
or a multi-net cut remains possible. The verified 65-net board is retained.

[Retained result and last-failure analysis](../../build/interf-restoration-2026-09-08/continued/analysis.html).

Both native sequence audits and all four repair trees pass independent checks,
including recursively audited child repairs and rejected alternatives. The routing executable is unchanged from
the recent-change study: this experiment exercises an existing mechanism with
an explicitly bounded recovery policy. No full-board completion, area
reduction, broad runtime improvement or external-router superiority is claimed.

## Compact analysis tool

`summarize_routing_recovery.py` turns the last sequential step and its nested
repair journals into `analysis.json` and `analysis.html`. It retains:

- Ordinary search failures, skipped guidance and recorded search work.
- Each repair target, protected nets, tested coverage and individual failed
  trial errors. Counterfactual routes remain distinct from admitted repairs.
- The net yielded by each attempt, restoration triggers, child repair chains,
  stopping bounds, final native counts and selection status.
- Links to complete journals, independent audits and combined copper views.

The output summarizes journal evidence; it does not independently verify a
board or determine process liveness. The caller must confirm command completion.
Unknown/missing repair journals remain explicit. The summary covers the last
attempted sequential step, not every historical repair.

The sequence audit now generates this report automatically. The writer also
creates combined front/back copper views when a provisional stage has only
its original preview, preventing broken image links and component bodies
obscuring the traces. Missing views are generated in a content-keyed
`analysis-views/` cache; immutable provisional inputs remain unchanged.
A native child source is checked byte-for-byte against its original snapshot
before and after report regeneration. Expandable details retain the deeper evidence without
filling the initial page with route candidates and geometry arrays.

Controls compare the compact output against the frozen rejected `PC-A4` repair
and the accepted three-net chain. They check protected-net identity, exact
changed-net receipts and the distinction between local progress and whole-board
completion. Headless Chrome verifies evidence links, loaded copper images,
all three net names, connection-limit status and expandable trial errors.
The retained screenshot was inspected.

The native placement-routing driver now supplies one nested invocation, one
level and eight diagnosis trials when rip-up is enabled and no restoration
policy is specified. Explicit `null` and custom restoration policies are
preserved; `--no-nested-restoration` disables supplying this default. Direct
library configuration remains unchanged. Configuration controls check these
cases and exact equality with the nested bounds used in this experiment.
The new default is not a new cold-routing completion claim.

Artifacts, configurations, executable hash and analyzer source snapshots are
retained under `build/interf-restoration-2026-09-08/`.

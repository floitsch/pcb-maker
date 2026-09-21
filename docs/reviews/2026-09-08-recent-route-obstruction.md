# Using retained history to find a routing obstruction

`interf_u` stopped at 57 of 110 nets with 140 native open items. `PC-A1` was
disconnected on both tested routing grids. The useful next question was which
retained copper closed its passage, rather than whether a larger search budget
could find a path.

The checkpoint immediately before adding `PC-A0` answers this directly:
`PC-A1` routes and passes native verification there. The existing sequence
audit proves that adding `PC-A0` was the only copper change between the two
checkpoints. `PC-A1` also routes on the empty board with the same placement and
rules. Routing `PC-A1` first and then restoring `PC-A0` succeeds natively,
reducing opens from 140 to 138 with no new findings.

[Combined comparisons and audited sequences](../../build/interf-obstruction-2026-09-08/index.html).

## Production change and bounded comparison

Yielding diagnosis accepts `preferred_yielding_connections`, an advisory order
for existing foreign routed nets. Unknown names are ignored and duplicates are
removed after the router's usual net-name normalization. The remaining nets
stay in lexical order. The hint changes which counterfactuals fit a small
budget; it does not remove the remaining search options or authorize deletion
of any real copper.

The sequential coordinator supplies most recently changed nets first, using
committed step history. An earlier repair's target, restored net and other
recorded changed nets are included. Uncommitted failures are excluded. Direct
diagnosis without hints retains lexical ordering.

Two native runs start from the same 57-net checkpoint with identical routing
settings, one repair invocation and one diagnostic trial:

| Diagnosis order | First yielded net | Result |
| --- | --- | --- |
| Previous lexical order | `8MH-OUT` | No counterfactual route; remains at 57 nets / 140 opens |
| Recent-change priority | `PC-A0` | Target routes and yielded net is restored; 58 nets / 138 opens |

`PC-A0` is 57th in the lexical inventory. This is a measured improvement in
which action a one-trial budget tests, not a claimed 57-fold runtime speedup.
Full-budget diagnosis still enumerates its bounded trial set before selecting
a restored result; this change does not make that enumeration lazy.

The repair uses the existing transaction: only the target and yielded net may
change, and restored connectivity, source dimensions, unrelated copper, poses,
source project and native findings are checked before commit. Both independent
sequence audits pass. Diagnostic schema 3 records the actual trial order, and
the independent audit reproduces that order from the stored hints and inventory.
The sequential checkpoint schema remains 7.

All 139 KiCad adapter tests and eight benchmark tests pass. Added tests cover
unknown/duplicate hints, retained foreign-net coverage, restored-net recency
and exclusion of uncommitted steps. Audit mutation controls reject missing
foreign nets, ignored hints and a trial that disagrees with the recorded order.

Every native probe and repair retains automatic combined front/back renders
without component bodies. The initial cold probe wrapper rejected a directory
without a verification report before routing; the retained `cold-verified`
probe uses the already verified cold snapshot.

## What the evidence does and does not establish

This case provides a causal recent-change diagnosis, followed by a verified
repair. It does not establish that the latest route is always responsible for
a failure or that one-net rip-up is sufficient for every obstruction. Earlier
barriers and multi-net cuts still need broader investigation. The board remains
incomplete, so no whole-board completion, area reduction or superiority over
external routers is claimed.

Artifacts, immutable inputs and binary/source snapshots are retained under
`build/interf-obstruction-2026-09-08/`.

## Broader history-window continuation

A fresh bounded continuation starts from the admitted 58-net checkpoint with
four repair invocations and eight diagnostic trials per invocation. It stops
at **62 nets / 131 opens**, with no native design/ERC/parity findings. The
independent sequence audit passes. Three further repairs commit:

- `PC-A11` routes after yielding and restoring `PC-A10`.
- `PC-A2` routes after yielding and restoring `PC-A1`, third in recent order.
- `PC-A3` routes after yielding and restoring `PC-A2`.

The next `PC-A4` trial finds a target path after yielding `PC-A3`, but restoring
`PC-A3` is grid-disconnected. The coordinator rejects that provisional result
and retains the verified 62-net board. This identifies a restoration-stage
obstruction; alternatives include another target path, an older blocker, or
a bounded repair involving another net. It is not evidence of physical-board
infeasibility.

All five new repair trees also pass the independent partial-rip-up audit,
including the rejected PC-A4 attempt: preserved unrelated copper and poses,
source dimensions, provisional verification, final native selection and
unchanged source snapshots.

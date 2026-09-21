# Boundary analysis now drives a committed compound repair

The sequential router now reproduces the previously experimental `interf_u`
repair from the committed 65-net checkpoint. It reaches **66/110 nets and 123
native open items**, down from 125, without new design/ERC/parity findings.
The independent sequential audit reproduces all 65 inherited boards exactly.
This is progress on an incomplete reference board, not whole-board completion.

[Evidence and combined copper views](../../build/interf-compound-recovery-2026-09-08/integrated/analysis.html).

## What the diagnosis taught the program

Removing one obstructing net was insufficient: routing the target could leave
the displaced net trapped. The optional `ripup.compound_recovery` policy now
tries a bounded family of two-net removals after ordinary and single-net repair
fail. Candidate pairs follow the existing recent-change priority and are ranked
by target length plus the configured via penalty. Both restoration orders are
tried. The first displaced net must route directly; the second may invoke
bounded boundary-guided repair, including the existing nested restoration.

On this checkpoint the 16 tested pairs produce two target routes. The lower
cost candidate displaces PC-A6 and MD2. Restoring PC-A6 first fails; restoring
MD2 first succeeds. PC-A6 then needs a repair that temporarily displaces PC-A7.
Fresh boundary evidence identifies PC-A2 as a useful additional change, allowing
all displaced connections to be restored. No net names are hardcoded in the
new mechanism. Initial pair priority still uses recorded routing history;
boundary priority is recomputed on the changed restoration board.

Matched nested controls with demand penalties zero and four both succeed.
The integrated run therefore uses zero forecast penalty. This does not show
that demand is generally unnecessary; it removes an unnecessary dependency
from this particular recovery.

## Transaction and evidence contract

Every native verification produces an automatic preview. The independent audit
adds combined front/back copper views without component bodies, retaining via
and component labels. Its intermediate views explicitly identify missing nets.

Only a result that passes admission against the **original committed parent**
can be promoted. The gate requires the expected drop in open items, no newly
introduced open-item fingerprints, native physical checks and preserved context
outside the declared changed nets. No intermediate displacement is committed.
The source directory is hashed and copied before the action; its immutable
snapshot remains available after the coordinator promotes its mutable result.

Nested changes replace earlier route receipts. This matters here: the final
PC-A7 route differs from the initial pair candidate. The audit compares full
segment/via multisets against native readback, including coordinates, layers,
widths and drills. It rejects four planted bad receipts: the stale initial
target, a moved segment, a wrong layer and a missing via. Final changed nets are
MD2, PC-A2, PC-A6 and PC-A7; unrelated copper, footprint/pad poses, project and
schematic content are preserved.

The compound audit checks pair ordering, both restoration orders, recursive
single-repair evidence, bounded call counts, and 741,657 recorded A* expansions.
That counter excludes obstacle-distance preparation, boundary flooding, native
verification and rendering; it is not a total runtime/work measure.

## Scope and controls

The optional configuration is disabled by default in the Rust API. The tested
policy permits 16 pair trials, two pair candidates, two restoration repairs,
four diagnosis trials per restoration, and one nested invocation at depth one.
Schema 8 journals retain the compound action and final changed-route receipts;
resume accepts schemas 3 through 8.

The compiled change passes 143 KiCad and eight benchmark tests. Native compound
and full sequential audits pass. The matched disabled run stops at 65 nets and
preserves its retained board exactly. Its single-net diagnosis and A* work match
the enabled run, excluding elapsed time and artifact locations.

Resume from the schema-8 checkpoint also passes: the next run reaches **67/110
nets and 121 opens**, repairing PC-A8 by rerouting PC-A4. Its independent sequence
audit preserves all 66 inherited commits and the original native constraints.
PC-A9 then fails grid connectivity; repair is skipped because the one-invocation
budget was consumed at PC-A8. A separate continuation allows four invocations
without changing the per-action limits. It finishes at 83 nets and 100 opens;
independent terminal audit passes with all 67 inherited commits preserved.
It repairs PC-A9 through PC-A4 and PC-RD
through the compound mechanism, then stops at PROG-. A matched PC-RD experiment
finds stronger single-net coverage with boundary priority; see the
[comparison](2026-09-08-history-versus-boundary-diagnosis.md).

The mechanism is deliberately bounded. It does not enumerate arbitrary removal
sets, prove infeasibility after exhaustion, or guarantee completion. Next work
should follow the next failed connection and broaden general recovery coverage,
rather than tune the vias of this partial result.

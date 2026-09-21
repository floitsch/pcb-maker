# Automatic nested routing recovery

The coordinator now handles the verified larger-board conflict without an
external sequence of repair commands. Starting from its 27-net checkpoint,
it inserts `Net-(C201-Pad2)`, yields `+12V`, and automatically invokes another
repair when `+12V` cannot be restored. That repair yields and restores
`Net-(U102-CAP+)`. The resulting **28-net / 34-open** board is byte-identical to
the preceding diagnostic result. Seven existing annotation findings remain;
there are no new native findings or disconnections.

## Contract

`ripup.restoration` is optional. Its `maximum_invocations` budget is shared by
all nested calls within one outer rip-up invocation; `maximum_depth` limits
nesting and `maximum_diagnosis_trials` bounds each nested diagnosis. Defaults
when enabled are one invocation, one level and 64 diagnosis trials. Supported
limits are 1–16 invocations, 1–3 levels and 1–1024 trials. Exhaustion is recorded
as a skip reason. A resumed sequential invocation receives a fresh outer repair
budget, as before; the bounds are not a lifetime limit across manual resumes.

An optional `routing` profile selects search parameters for restoration. The
current native connection rules and remaining-net demand are inherited from the
enclosing repair. Restored ancestor targets are protected from being yielded
again, and their demand forecasts are removed. The tested profile uses 0.25 mm
resolution and multi-source tree attachment; the enclosing insertion retains
its existing 0.5 mm profile.

Every level must restore its displaced net and pass its own native admission.
Before promotion, the enclosing level also checks its original open-item ledger,
unchanged remaining disconnections, native findings, project/ERC inputs, fixed
geometry and every copper object outside the declared changed-net set. Child
candidates stay in their own directories. Only the admitted board, reports and
combined preview are copied into the parent trial. A failed recovery never
changes the coordinator's retained board.

Each admitted repair carries candidate hashes and quality receipts for **all**
changed nets. The adaptive ordering policy uses the final quality of every
replaced net, and native readback audits use each candidate's recorded rules.
Schema-6 sequential checkpoints retain these receipts; resumption validates their
hashes and identities in addition to fresh native verification of the board.
Older supported checkpoints remain readable. The repair audit now reads the
immutable `source-unrouted-target` snapshot: the coordinator can legitimately
change its result directory after that repair commits. Exact snapshot directory
hashes independently check the recorded action inputs. Failed direct-restoration A* work
is now included in the recorded expansion total.

## Controls

- Automatic recovery preserves all 27 inherited committed boards exactly and
  advances to the known 28-net board. Independent audits check every inner and
  outer restoration alternative, selection, dimensions and unchanged copper.
- Limiting the child diagnosis to one candidate prevents restoration. The
  sequence terminates incomplete with the original 27-net board byte-identical.
- Validation-only resumption accepts the 28-net checkpoint and adds no routes.
  A changed candidate file with the old receipt hash is rejected explicitly.
- Enabled-but-unused nested recovery completes all nine ECC83 nets and
  reproduces every earlier board in that sequence byte for byte.
- All 129 KiCad library tests and eight benchmark tests pass, including shared-budget/depth behavior and
  adaptive ordering after a nested repair replaces an earlier route.

[Combined report and step viewers](../../build/nested-recovery-2026-09-08/index.html)
retain all native results, source/executable hashes and process exit records.
The first continuation from 28 nets reaches **31 nets / 28 opens**. It inserts
`Net-(C202-Pad1)` through another nested `+12V` repair, this time restoring
`Net-(R312-Pad2)`, then routes `Net-(C301-Pad2)` and `Net-(C302-Pad1)` normally.
It stops at `ampli_ht_horizontal/PIEZO_OUT` after using its one configured outer
repair invocation. The subsequent continuation uses four outer invocations as its allowance and
finishes at **34 nets / 24 opens**. It repairs the horizontal piezo output and
`Net-(Q204-E)`, with an ordinary `Net-(C204-Pad1)` insertion between them.
All seven existing annotation findings remain and full-board completion is false.
Across these three resumed invocations, the board advances from 27 to 34 nets;
this is not a claim of seven new nets within a single four-repair budget.

The next target, `Net-(R212-Pad2)`, has six successful outer yielding
counterfactuals, but none restores directly. The one allowed nested invocation
is spent on `-VAA`. That child diagnosis checks 28 of 34 eligible nets and only
finds the protected ancestor target as a successful removal, so it has no
admissible restoration attempt. Five sibling candidates do not receive nested
search because the shared budget is exhausted. Three of four outer invocations
were used: this stop is a nested search-budget/coverage boundary, not an
exhaustive repair failure or evidence that placement must change. The next
experiment should cover the remaining child candidates and sibling restorations
before drawing that conclusion. No complete larger board is claimed.

The larger-board benchmark now enables one bounded nested restoration per outer
repair, with the tested finer search profile. This is still a two-layer routing
capability. Completion across arbitrary board families and broader placement
recovery remain open work; local via optimization stays deferred.

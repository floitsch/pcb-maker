# Dual-ESP32 thin-route width continuation

This experiment tests a specific composition of the two predecessor ideas:
use the DUT grid router for discrete topology, then use the continuous
push/pull engine to grow copper without changing topology. It is not a claim
that thin traces are manufacturable, and no intermediate state is promoted as
a finished board.

## Contract

Each run starts from the prefix-19 semantic problem, regenerates harmonic
placement, starts with zero copper, and routes all 24 semantic branches. The
seed scales trace widths only; clearance remains at its declared full value.
The continuation stages then increase every trace toward its declared width.
When a stage is invalid, only the continuous engine may move the selected
traces/components. It may not call A* or otherwise change topology. Every
retained stage must pass the independent exhaustive semantic geometry and
electrical gates. A final validation forcibly restores declared widths, so a
thin candidate cannot report full completion.

The run retains the thin seed, each exact stage, every rejected proposal,
selection/work telemetry, and paired front/back renders. A full-width result
would still need KiCad ERC, DRC, schematic-parity, and connectivity admission;
none of these runs reached that boundary.

## Result

The ordinary connection order at 20% width routes 22/24 branches. Giving
`SIG06_W_R` and `SIG07_W_R` priority and starting at 5% produces a complete
24/24 topology in 0.636 seconds. This is a substantial capability result,
unlike the earlier millimetre-scale copper comparisons: prefix 19 has a
complete discrete topology for the first time.

The first adapter attempt then exposed unsupported trace/pad and via/trace
findings. Fixed existing vias are now lowered as zero-length circular
polylines with shared anchors; mixed-layer routes are split into same-layer
runs; component-pad findings conservatively select the owning component body;
and selected motion closes over every incident branch. Intermediate controls
also exposed a too-small 100,000-pair work bound, accidental f32 rewrites of
fixed geometry, and incomplete selected/context coupling. Those runs remain
in the journal because they explain the adapter changes rather than being
discarded as bad scores.

Sequence 181 is the clean instrumented trace-only ablation:

| Stage | Before engine | Engine result | Work |
| --- | --- | --- | --- |
| 5% seed | 24/24, exact | no engine needed | discrete topology only |
| 10% width | 1 finding, 0.003483 mm total shortfall | exact after moving one of two selected traces; no component motion | 82,303 trace pairs, 432 trace/body pairs, 17 frames, 84,724,736 projections |
| 15% width | 3 findings, 0.016748 mm total shortfall | rejected: 20 findings, 3.221689 mm total shortfall | 142,002 trace pairs, 1,750 trace/body pairs, 17 frames, 147,208,192 projections |

The rejected 15% proposal contains 13 trace/obstacle findings, six
trace/trace findings, and one trace outside the board. Its maximum constraint
residual is 0.219865 mm. The trace-only run still selected one body as context,
but moved no bodies. A coupled-body run and a one-frame/no-tension run fail in
the same way, so component mobility, extra frames, and trace tension are not
the sole cause.

The positive conclusion is narrow but important: starting thin converts the
prefix-19 problem into a complete topology, and the engine can grow that
topology through one exact width stage. The negative conclusion is also
specific: the current projector has no trust region or topology-preserving
barrier, so it can trade three small local conflicts for a much worse global
state. This does **not** reject width continuation or component squeezing.

## What the experiment changed

`pcb-routing` now exposes a replaceable width-continuation coordinator. The
continuous candidate adapter additionally handles mixed-layer runs, fixed
existing vias, trace/pad ownership, and incident-branch closure for this
experiment. The CLI command is:

```sh
target/release/pcb-maker place-route-grid-width-continuation \
  problem.json placement-config.json width-continuation-config.json output-dir
```

The configs remain separate from the coordinator. Alternative seed routers,
width schedules, continuous processors, selection rules, and later discrete
repair producers can therefore be compared without baking this prefix's
answer into the architecture.

## Next experiment

Sequences 182–188 implement and evaluate that next step. The semantic trust
region caps each trace point, body translation, and body rotation independently,
tries a bounded halving schedule, and exact-gates every trial. The unbounded
processor remains selectable as a control. A two-phase continuation now tries
trace-only motion first and recruits connected component bodies only when that
processor cannot improve the exact state; both attempts remain in evidence.

The adapter now lowers every fixed rectangular pad as an oriented body and
every fixed circular pad as a zero-length circular polyline. Own-terminal pads
are exempted by exact component/pin ownership. This fixes the proxy defect that
stopped sequence 181:

| Sequence | Result | Diagnosis |
| --- | --- | --- |
| 182 | exact 10%; 15% shortfall decreases monotonically but only by 0.000635 mm in 16 rounds | one global scale lets a distant 14 mm raw motion suppress useful local motion |
| 183 | exact 10%; first local step removes both via/trace conflicts and leaves one pad conflict | per-point caps work; the component-body pad proxy does not |
| 184 | exact through 20%; 30% stops before motion | exact rectangular pads work; a circular connector pad was still unsupported |
| 185 | exact through 20%; 30% proposal rolls back | fixed circular-pad lowering works; the 10 percentage-point width jump creates a broad conflict set |
| 186 | exact through **25%**; 27.5% reduces 99 findings to 90, then stalls | the remaining 90 segment findings belong to only four trace pairs |
| 187 | component coupling also reaches 25%, but early body motion creates a worse later basin | component freedom is useful but should not precede a successful trace-only repair |
| 188 | two-phase trace-first/component-fallback reaches 25%; fallback changes 90 findings to 94 and rolls back | this is the first genuine bounded-engine stall suitable for the outer loop |

Sequence 188 keeps all 24 branches and exact-passes every retained stage from
5% through 25%. At 27.5%, the trace-only processor improves 99 findings/
0.446051 mm shortfall to 90/0.402879 mm. Those findings reduce to four route
pairs: `SIG02_W_R` against `SIG05_R_E` and `SIG05_TAP_R`, plus `SIG04_R_E`
against the same two `SIG05` routes. The connected-component fallback is worse
(94 findings/1.288669 mm) and is rejected. Forced full-width validation still
has 502 findings, so this remains semantic continuation evidence rather than a
native-complete board.

Sequences 189–192 implement the first outer-loop experiment. The four pressure
pairs form a K2,2 conflict graph, so the coordinator enumerates its two minimum
vertex covers and selectively reroutes each from the same retained parent.
Sequence 189 exposes an over-broad shared-tree guard. After narrowing that
guard to genuinely generated shared trees, sequence 190 reaches 23/24 by
rerouting `SIG02_W_R` plus `SIG04_R_E`, and 22/24 by rerouting both `SIG05`
branches. Sequence 191 retains the detailed failures: `SIG02_W_R` is a true
`no_path` on both 0.25 and 0.125 mm grids after 4.086 million expansions, not a
budget exhaustion. Sequence 192 extends failed covers with high-pressure
routed-trace blockers and exposes a terminal-via lowering mismatch before the
next continuation stage.

The external sequence-193 baseline proves the fixed placement is routable at
the exact 0.25 mm trace and 0.70/0.30 mm via rules: Freerouting completes 24/24
in seven passes and 5.90 seconds, with zero KiCad copper/connectivity findings.
See [`dual-esp32-freerouting-baseline.md`](dual-esp32-freerouting-baseline.md).
The priority is therefore connectivity-safe multi-pass rip-up/retry. Exact
movable compound pad/keepout children and relational placement constraints
remain important, but more A* budget, finer score comparisons, or more
unconstrained engine frames are not the next lever.

## Sequence journal

- 166: the generic cold order search rediscovers the prefix-18 swap and
  completes 18/18; the hand-authored order is no longer required.
- 167: the first prefix-19 search fails the initial 64-sweep placement gate.
- 168: longer legalization evaluates three cold orders; all stop at rung 14.
- 169: targeted placement/order variants stop at rungs 14, 14, and 11.
- 170: ordinary-order 20%-width seed routes 22/24, so the engine correctly
  does not run on an incomplete topology.
- 171: prioritizing the two failed branches routes 24/24 at 5%; the first
  growth stage reports unsupported finding types.
- 172: fixed-via/trace-pad support reaches the explicit 100,000-pair ceiling.
- 173: a two-million-pair ceiling runs the engine and exposes selected/context
  coupling defects.
- 174: incident-branch closure removes one coupling defect.
- 175: exact no-op writeback removes false electrical failures; the remaining
  regression is geometric.
- 176: fine stages first exact-grow the topology to 10%, then reject 15%.
- 177: one frame with zero tension also regresses at 15%.
- 178: an accidental debug-build run was aborted and has no evidence render.
- 179: the release trace-only ablation exposes fixed-geometry f32 rewrite.
- 180: exact fixed-geometry writeback leaves only real geometry failures.
- 181: instrumented repetition of 180 confirms the 10% success and quantifies
  the 15% failure above.
- 182: global trust-region control; safe monotone motion but negligible gain.
- 183: per-point trust region removes two of three 15% conflicts.
- 184: exact rectangular-pad lowering grows through 20%.
- 185: circular-pad lowering removes the next unsupported boundary.
- 186: finer continuation exact-grows through 25% and stalls at 27.5% on four
  trace pairs.
- 187: component coupling from the first trace conflict reaches the same width
  but creates a worse later basin.
- 188: trace-first/component-fallback composition preserves the exact 25%
  lineage and retains both failed 27.5% processors.
- 189: minimum conflict-cover enumeration is blocked by an over-broad
  shared-tree selective-reroute guard.
- 190: both minimum covers run but stop at 23/24 and 22/24.
- 191: detailed blocker evidence proves the leading failure is exact
  `no_path`, not budget exhaustion.
- 192: bounded blocker-frontier expansion exposes terminal-via lowering.
- 193: exact-rule Freerouting baseline completes the same fixed-placement
  board at 24/24 in seven passes.

All sequence-181–193 front/back stages and rejected proposals are copied to
`build/progress/` for visual inspection.

# Immutable-parent connection insertion

This port combines the strongest parts of both predecessors without making
their engines interchangeable. `layout-trace` supplies the transaction shape:
one exact declaration delta, an immutable certified parent, local repair first,
whole-target certification, and exact rollback. The current Rust DUT router
supplies bounded local A*, while the pressure coordinator is the global
router-to-placer fallback.

The public V1 operation is:

1. require source and target to differ by exactly one legacy two-terminal Net;
2. independently exact-validate the parent under the source;
3. run each admitted local policy from that same parent, with only the new
   branch mutable;
4. commit the shortest exact-complete local child, or run coupled global
   pressure repair with all legal poses and routes mutable;
5. return only an exact-complete child or the byte-identical parent.

Failed candidates never cross the result boundary. Evidence retains attempt
scope/status, bounded work, blocker IDs, changed routes/vias, moved components,
whole-target counts, and canonical hashes binding config, source, target,
inserted net, parent, and committed child.

## Progressive ESP32 result

The predecessor's five stages were copied byte-for-byte. Stage two immediately
rolled back even though all three local trials found collision-free geometry:
both sides of `R_A` were labeled `SIGNAL_A`, so the independent electrical gate
reported two connectivity findings. A resistor is not copper. The historical
files remain under `benchmarks/imported/`; the native control removes only the
cross-resistor electrical aliases.

All four additions in the corrected control then commit locally. The last
direct witness crosses retained copper, but local A* finds an exact top-layer
detour without moving a component, changing an old branch, or adding a via:

| Branch | Final length (mm) | Points | Vias |
| --- | ---: | ---: | ---: |
| `A_WEST` | 5.288856 | 10 | 0 |
| `A_EAST` | 9.844169 | 17 | 0 |
| `B_WEST` | 8.627004 | 16 | 0 |
| `B_EAST` | 16.068297 | 30 | 0 |

The final branch is deliberately not evidence for global repair: it is still
easy enough to detour locally.

## Local portfolio ablation

The initial default tried a base policy, a four-times-via-cost policy, and a
fine-grid retry policy. All three returned the same exact child hash
`sha256:1be43dd266bc3eba7f1a7c49beb2cc0834031ebbdd792865ef2d4834f2b38e48`.
One policy used 946 expansions; three used 2,838. Because no trial needed a via
or refinement, the extra policies bought no diversity. V1 therefore keeps the
portfolio configurable but defaults to one policy. The checked-in
`three-policy-ablation.json` reproduces the negative result.

## Coupled fallback control

The asymmetric wall channel provides the missing escalation. Local routing at
0.5, 0.25, and 0.1 mm has no complete fixed-parent route after 432,220
expansions. Global pressure evaluates the original placement and one coupled
generation, moves `WALL` from y=15.0 to y=14.5, reroutes `SIGNAL`, and commits
after 436,739 total expansions. No incomplete geometry becomes the selected
state.

Sequence 041 in `build/progress/` contains thirteen paired front/back stages:
the historical semantic rollback, corrected progression, portfolio ablation,
and global-pressure commit. Back renders are intentionally empty because these
controls declare only top copper.

## Native KiCad transaction

The same boundary now reaches the real single-ESP32 ladder through
`insert-kicad-connection`. The operation reads the exact materialized parent,
makes a byte-identical snapshot, verifies a disposable copy, and derives the
next schematic and pad connectivity from the declaration. It then hydrates
that connectivity onto the parent's exact component poses, graphics, and
existing copper. The external parent is never passed to KiCad.

Before routing, a native baseline must isolate exactly the intended delta:
ERC, design DRC, and schematic parity are all zero while the newly selected
net is unconnected. Every local portfolio entry starts from that same
unrouted target. A child is eligible only after native ERC, DRC, parity, and
connectivity all pass. Every failed search, application error, native
rejection, candidate, report, and artifact directory remains inspectable.

The rung 0 to rung 1 experiment adds `T_LOCAL_COMM0`:

| Result | A* expansions | Copper (mm) | Vias | Native result |
| --- | ---: | ---: | ---: | --- |
| New local child | 110 | 19.617588 | 0 | complete, committed |
| Historical declared target | n/a | 19.242641 | 0 | complete, control only |
| One-expansion local control | 2 before rejection | n/a | n/a | route failed |

The local child is 0.374947 mm (1.95%) longer than the historical route. That
is a quality gap, not a correctness failure: it was independently derived
from the empty parent and passes the whole native pipeline. With the search
budget reduced to one and fallback disabled, the selected rollback directory
has the same directory hash as the parent snapshot. With the historical
control enabled, KiCad proves that the target itself is feasible, but the
transaction still rolls back. A declared target is not a parent-preserving
repair and therefore cannot masquerade as a globally discovered child.

Sequence 042 retains the initial migration failure (an older report lacked the
new additive `pcb_zones` field), the local commit, the deliberately exhausted
attempt, the exact rollback, and the historical control as paired front/back
images. Additive report fields are now backward-compatible; semantic board,
rung, and connection-prefix checks remain strict.

## Coarse generated-parent progression and the first rip-up repair

`progress-kicad-connections` repeats the transaction with one crucial rule:
only the selected child becomes the next parent. It writes a `running`
checkpoint after every completed step, stops at its explicit bound, and stops
immediately on rollback or execution failure. Historical rungs are measured
for quality but never enter the parent chain.

The first generated-parent run commits rungs 1–12. Rungs 1–8 use 27,991 A*
expansions and total 180.421277 mm for their newly inserted traces versus
186.605757 mm historically. They use no vias versus four historical vias.
This is not uniformly better: three early traces are 0.87–2.87% longer, while
others are up to 9.54% shorter.

The 0.25 mm control's first apparent failure occurs at rung 12 → 13.
`T_LOCAL_FLEX3` has no route against the exact generated parent; the base
search reports no path after 77 expansions. The historical rung-13 route
remains native-complete. At this point that distinguished fixed-copper failure
from target infeasibility, but did not distinguish real topology closure from
conservative rasterization. The resolution ablation below resolves that
ambiguity.

The bounded yielding diagnostic removes only foreign tracks/vias while
retaining every pad as an obstacle. Ten single-net counterfactuals leave the
same coarse-grid failure. Two make that coarse search routable:

| Yielding connection | Target route | Expansions | Vias |
| --- | ---: | ---: | ---: |
| `T_LOCAL_FLEX0` | 18.589694 mm | 148 | 0 |
| `T_LOCAL_FLEX1` | 19.478781 mm | 4,339 | 2 |

Those are diagnostic states, not boards. The single-connection rip-up
producer therefore applies each target route, native-checks that only the
yielded net is missing, reroutes that old net around the target, and runs the
whole native gate again. Both actions complete. The selected action yields
`T_LOCAL_FLEX0`, reroutes it as 19.617588 mm/two vias, and has a combined
via-penalized score of 42.207282 mm. Yielding `T_LOCAL_FLEX1` scores 47.095691
mm. The automatic insertion transaction selects the first action after 3,787
expansions in its two selected searches; the complete diagnosis plus both
restoration attempts costs 11,243 expansions. It produces the same board bytes
as the standalone experiment.

That repaired rung becomes an ordinary parent. The next bounded coarse run
reaches rung 21 with all eight additional children native-complete.
Correctness does not imply quality: `RELAY0`, `RELAY1`, and
`T_PROBE_XDATA_0` are respectively 158.34%, 67.38%, and 39.62% longer than
their historical routes. The long detours are visible rather than hidden.

Sequence 043 contains paired front/back images for the empty parent, every
generated rung through 21, the rung-13 failure, historical control, exact
rollback, both target-only counterfactuals, both restored boards, and the
automatic repair commit.

## Resolution ablation corrects the diagnosis

Retained-vertex shortening is not the fix for `RELAY0`. Both the greedy and
visibility-shortest-path processors reduce 39.553309 mm to only 39.308047 mm
and remove two bends. They cannot change the retained topology. More
importantly, the 15.310660 mm historical route applies to the exact generated
parent with zero native findings. This proves that a valid short corridor
exists and that the coarse grid hid it.

Running the same parent-derived A* producer at finer resolution makes the
difference explicit:

| `RELAY0` policy | Length | Expansions | Native result |
| --- | ---: | ---: | --- |
| 0.25 mm | 39.553309 mm | 105,662 | complete |
| 0.125 mm | 7.183029 mm | 2,907 | complete |
| 0.10 mm | 6.705185 mm | 2,965 | complete |

The finer search is both shorter and cheaper here because it enters the local
corridor instead of exploring a board-scale detour. The same control overturns
the rung-13 interpretation: fixed-parent 0.125 and 0.10 mm searches route
`T_LOCAL_FLEX3` in 17.642655 and 17.660009 mm with two vias, and both boards
pass the native gate. The old rip-up transaction remains valid evidence that
the fallback can restore a board, but it was not necessary for this case.

A fresh progression from the empty board then evaluates 0.25, 0.125, and
0.10 mm at every rung and ranks only native-complete children by physical
copper plus via penalty. All 63 candidates pass, and all rungs 1–21 commit
without invoking rip-up. Two selected children use 0.25 mm, seven use
0.125 mm, and twelve use 0.10 mm. Selected routes total 477.767901 mm and 14
vias versus 553.350268 mm and 22 vias historically, a 75.582367 mm (13.66%)
length reduction. The selected searches consume 861,304 expansions; measuring
the complete three-way portfolio consumes 2,283,335.

The former quality regressions reverse: `RELAY0`, `RELAY1`, and
`T_PROBE_XDATA_0` become 56.21%, 74.53%, and 6.07% shorter than their
historical controls. Sequence 044 retains the shortening controls, historical
route diagnostic, fine-grid `RELAY0` and rung-13 boards, and every fresh
generated rung as 30 paired front/back stages.

## Score-ordered native admission

The exhaustive portfolio establishes an important premise: on all 21 rungs,
the selected child is the lowest-scored generated candidate, and all 63
candidates pass KiCad. The transaction now exposes two admission policies so
that this premise is not silently generalized:

- `exhaustive` native-gates every generated candidate and remains the control
  for comparing algorithms;
- `score_ordered_until_complete` ranks all generated candidates by the same
  physical-copper/via/cost/ordinal rule, then native-gates candidates until the
  first complete child.

Lower-ranked candidates in the second policy retain their route, quality,
work, and artifact directory with status `candidate_generated`, no verification
report, and `native_gate_invoked: false`. They are evidence, not accepted
boards. If a better candidate is rejected, the deterministic admission order
continues to the next candidate; topology fallback starts only after every
generated local candidate has failed admission.

Sequence 045 regenerates the full rung-0-to-21 experiment with score-ordered
admission. It produces the same 2,283,335 A* expansions and all 63 route
candidates, but performs 21 candidate gates instead of 63. Every selected
route JSON and every selected PCB is byte-identical to sequence 044. The result
still totals 477.767901 mm/14 vias. The run took 12:04.88 wall time, but there
is no matched timed exhaustive run, and repeated baseline gates,
materialization, KiCad startup, and A* work remain; the gate-count reduction is
not presented as a proportional runtime claim. All 22 empty-to-rung-21 states
are retained as paired front/back images.

## Adaptive ordered resolution

Score-ordered native admission removes redundant KiCad gates, but still runs
all three A* searches. The next experiment treats the routing portfolio as an
ordered coarse-to-fine sequence. It always evaluates the first two entries
(0.25 and 0.125 mm). The 0.10 mm entry is evaluated only when the newest live
candidate improves the previous best score by at least 3%. A failed route is
not convergence, and the decision never reads a future or historical result.

The fresh adaptive chain commits all rungs 1–21 locally. Fourteen rungs stop
after two searches; seven evaluate all three (rungs 3, 13, 15, 17, 18, 19,
and 21). No prefix required recovery in this progression. The measured result
is:

| Policy | Candidates | KiCad candidate gates | A* expansions | Copper | Vias |
| --- | ---: | ---: | ---: | ---: | ---: |
| Exhaustive three-resolution control | 63 | 63 | 2,283,335 | 477.767901 mm | 14 |
| Score-ordered gates, full routing portfolio | 63 | 21 | 2,283,335 | 477.767901 mm | 14 |
| Adaptive ordered routing and score-ordered gates | 49 | 21 | 1,738,824 | 486.082655 mm | 16 |

Adaptive scheduling saves 544,511 expansions (23.85%) but adds 8.314754 mm
(1.74%) and two vias relative to exhaustive refinement. With the configured
2 mm via penalty, its aggregate score is 2.43% worse. It still uses
67.267612 mm (12.16%) less copper and six fewer vias than the historical
routes. The run took 10:26.36 wall time including compilation; this is retained
as an observation, not a general runtime claim. Its selected searches account
for 1,011,583 expansions and the unselected portfolio work for 727,241.

Stopping is deliberately fail-closed. A separate rung-0→1 control uses two
0.05 mm prefix routes that the producer can generate but KiCad rejects. The
scheduler first stops after those two candidates, admits and rejects both,
then restores the skipped ordinary-width 0.10 mm entry. That third candidate
passes and commits. Evidence records `evaluated_before_stop: 2`, three entries
ultimately evaluated, three native gates, and
`resumed_after_native_rejection: true`. Sequence 046 contains every adaptive
progression state; sequence 047 contains both rejected prefix boards and the
recovered child, all paired front/back.

## One-step continuation to rung 22

The first post-rung-21 experiment deliberately returns to exhaustive
generation and native admission. Starting from the better full-refinement
rung-21 parent, it adds exactly `T_PROBE_XDATA_1`; it does not launch a long
run toward the harder end of the board. All three local children pass KiCad:

| Resolution | Expansions | Copper | Vias | Score |
| --- | ---: | ---: | ---: | ---: |
| 0.25 mm | 47,332 | 24.941781 mm | 4 | 32.941781 mm |
| 0.125 mm | 127,820 | 24.443322 mm | 4 | 32.443322 mm |
| 0.10 mm | 176,474 | 24.250674 mm | 4 | 32.250674 mm |

The fine candidate commits rung 22 after 351,626 aggregate expansions. Its
four vias have no sub-1 mm close pairs or clustered members. The historical
route is 22.995807 mm/four vias, so the generated child is 1.254867 mm (5.46%)
longer. This extends the native generated-parent capability boundary while
retaining a concrete quality target. Sequence 048 pairs the parent and all
three candidates front/back.

## First multi-terminal insertion: rooted star versus shared copper

Rung 22 → 23 adds the four-terminal `XDATA` net as three branches. The
historical rooted-star adapter routes every branch independently from the same
terminal. All three candidates pass KiCad, but the selected 0.10 mm child is
25.487881 mm/four vias and contains one sub-1 mm via pair (two clustered
members). It is 1.410572 mm (5.86%) longer than the historical zero-via route.
Sequence 049 retains the parent and all three rooted-star controls.

The adapter now has an explicit `shared_copper_tree` policy. It routes the
first branch normally, ranks already-owned grid points for each later branch,
trims a result through its last existing-tree contact, and reserves only the
selected branch. Every attachment attempt records its ranked search start,
actual trimmed join, layer, work, length, vias, and selection. The selection
objective and search count remain configuration, so the rooted star is still
available as a control.

With eight attachment searches per later branch, all three shared-tree
candidates pass KiCad. The 0.25 mm candidate commits at 22.404598 mm/two vias,
with 7.198 mm minimum via spacing and no clustered members. Relative to the
selected rooted star it removes 3.083284 mm (12.10%) and two vias; relative to
the historical route it is 1.672712 mm (6.95%) shorter but still has two more
vias. The first branch owns those two transitions and both later branches join
F.Cu copper without a via. Sequence 050 contains all paired renders.

The retained attachment evidence showed that several nearby search starts
collapsed to the same actual join after prefix trimming. A 1 mm same-layer
source-diversity ablation therefore tested whether those duplicates wasted
work. It produced byte-identical boards and the same quality at every
resolution, but increased total expansions from 65,812 to 192,451 because the
seven losing searches became farther and more expensive. This is a negative
result, not a promoted policy; sequence 051 retains it.

In this case rank zero wins both later branches at all three resolutions.
Restricting the portfolio to that nearest source reproduces every sequence-050
PCB byte-for-byte and reduces aggregate routing work to 44,744 expansions,
32.01% below the eight-source run and 76.75% below the diversity ablation. All
three children independently pass ERC, DRC, parity, and connectivity. Sequence
052 retains the parent and all candidates.

A separate checked-in-parent regression starts from historical rung 22 rather
than the generated rung-22 child. The same nearest-only 0.25 mm shared-tree
policy completes `XDATA` in 372 expansions at 23.111704 mm with zero vias,
0.965605 mm (4.01%) shorter than the declared historical route. This proves
that shared-tree growth itself does not require the generated parent's two
vias; that remaining gap depends on earlier copper and the first-branch route.
Sequence 053 pairs that parent and child front/back, and the native regression
is exercised by `native_shared_tree_insertion_reuses_copper_without_a_via_cluster`.

## One-step continuation to rung 24

Starting from the selected generated rung-23 parent, the next exhaustive step
adds only the two-terminal `D_BUS_DHT` connection. All three resolutions pass
the native gate:

| Resolution | Expansions | Copper | Vias | Score |
| --- | ---: | ---: | ---: | ---: |
| 0.25 mm | 14,033 | 40.227989 mm | 2 | 44.227989 mm |
| 0.125 mm | 47,896 | 40.164317 mm | 2 | 44.164317 mm |
| 0.10 mm | 71,761 | 40.108225 mm | 2 | 44.108225 mm |

The fine child commits rung 24 after 133,690 aggregate expansions. Its two
vias are 37.545 mm apart and have no clustered members. The historical route
is 39.794679 mm/two vias, leaving a small 0.313546 mm (0.79%) generated-parent
quality gap. Sequence 054 pairs the parent and all three candidates front/back.

## One-step continuation to rung 25

The next isolated step adds the two-terminal `DHT` connection. All three
native gates pass after only 70, 142, and 176 expansions. The selected 0.10 mm
child commits rung 25 at 9.093769 mm with zero vias, 0.136663 mm (1.53%)
longer than the 8.957107 mm historical route. Sequence 055 pairs the parent and
all three candidates front/back.

## Isolated continuation through rung 29

Three more one-connection transactions remain native-complete at all three
ordinary resolutions:

| Rung | Connection | Selected resolution | Expansions, all resolutions | Copper | Vias | Versus historical |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 26 | `D_BUS_ONEWIRE` | 0.25 mm | 183,273 | 20.265706 mm | 2 | −11.44% |
| 27 | `ONEWIRE` | 0.10 mm | 388 | 9.110819 mm | 0 | +1.72% |
| 28 | `D_BUS_US_TRIG` | 0.25 mm | 94,982 | 18.145399 mm | 2 | +2.65% |

Their selected via-bearing routes have no close pair. Sequences 056–058 pair
every parent and coarse/medium/fine child front/back.

At rung 28 → 29, ordinary refinement gives a deceptive result. The 0.25 mm
`US_TRIG` route is 35.299061 mm, while 0.125 and 0.10 mm apparently converge
at 12.937275 and 12.884495 mm. All pass KiCad, but the selected route is 65.02%
longer than the 7.807888 mm historical route. Sequence 059 retains the parent,
all candidates, and the historical board.

Applying the imported historical `US_TRIG` geometry to the exact generated
rung-28 parent produces ERC 0, DRC 0, parity 0, and zero selected-net
unconnected items. Sequence 060 retains the generated detour beside that
native-complete counterfactual, proving the gap is a router/raster miss rather
than parent congestion.

A single 0.05 mm search then finds the east-side channel in 12,604 expansions.
Its 6.648961 mm, zero-via board passes the full native gate and is 1.158927 mm
(14.84%) shorter than historical. Sequence 061 pairs its exact parent and
selected child. This is a stronger warning for adaptive scheduling than the
rung-13 case: adjacent 0.125/0.10 mm scores can agree while a materially better
finer corridor remains hidden. The sequence-061 child supersedes the
sequence-059 detour as the generated rung-29 parent.

## Continuation through rung 31

`D_BUS_US_ECHO` commits rung 30 at 17.726479 mm/two vias with the 0.125 mm
candidate, 1.41% longer than historical. `US_ECHO` then exposes three very
different ordinary-resolution topologies: 30.357693 mm/zero vias at 0.25 mm,
18.146263 mm/two vias at 0.125 mm, and 7.176043 mm/zero vias at 0.10 mm. The
fine child commits rung 31, 8.09% shorter than historical. Sequences 062 and
063 pair every parent and candidate front/back.

## `T_EN`: fixed-copper failure, repair, and fine shared tree

`T_EN` has four terminals and three routed branches. From the exact generated
rung-31 parent, rooted-star 0.25 mm routing fails after 676,118 expansions and
both 0.125/0.10 mm searches exhaust 2,000,001 expansions. Shared-tree routing
has identical failures because branch zero cannot form a trunk. The configured
one-old-net fallback finds three native-complete repairs by yielding
`D_LOCAL_FLEX1`, `T_BUS_RELAY1`, or `T_PROBE_XDATA_0`; it selects the last.
That repaired `T_EN` route is 57.875732 mm/six vias and has one close pair.
Sequences 064 and 065 retain the full rooted repair portfolio and the matched
shared-tree result.

The original artifacts underreported this run as 736,198 expansions. Failed
searches used the phrase `aggregate expansions`, while the evidence parser
accepted only the shorter suffix, so 32 failed searches were omitted. The
retained errors reconstruct 62,676,149 failed expansions; successful
counterfactuals and reroutes add 736,198, for 63,412,347 actual expansions.
The parser and local-attempt accounting now accept both forms, with a unit
regression. This changes work evidence, not candidate selection.

The 69.217641 mm/four-via historical `T_EN` tree does not apply to the
generated parent. KiCad reports seven design errors against
`T_PROBE_XDATA_0`, `T_BUS_RELAY0`, and `T_BUS_RELAY1`, confirming a real
topology conflict rather than the `US_TRIG` kind of quality-only miss.
Sequence 066 retains the unrouted parent and invalid historical control.

Nevertheless, fixed copper is not truly infeasible. A 0.05 mm rooted-star
search commits locally after 860,694 expansions at 57.088686 mm/six vias,
17.60% shorter than historical, though it retains one 0.721 mm via pair.
Sequence 067 contains that correction. At the same resolution, nearest-only
shared-tree growth attaches later terminals to the first trunk, cuts work to
334,597 expansions and copper to 51.008683 mm, and removes the close pair. It
uses eight vias, so the result is a tradeoff rather than a universal
improvement; with the declared 2 mm/via penalty its score is 67.008683 versus
69.088686 for the rooted star. Sequence 068 is the selected rung-32 parent.

An eight-source shared-tree portfolio tests whether another join removes the
six-via long branch. All seven alternatives trim to the same actual join and
route, producing a board byte-identical in quality to sequence 068 while
spending 2,640,195 expansions. Sequence 069 retains this negative result; it
does not justify a wider default.

## Boundary and next experiment

The native transaction still inserts one declared connection per step. The
adaptive result is one threshold point, not a tuned or generally optimal
policy. Its quality loss shows that the saved fine searches sometimes matter;
future scheduling should use board state, corridor evidence, or an explicit
work/quality budget rather than promoting 3% as a default. Its
parent-preserving topology fallback may reroute one old connection, but no
genuine fine-grid topology failure has exercised it yet. It does not compare
the opposite routing order, reroute two old nets, move a component, or shorten
the resulting pair through the continuous engine. The first multi-terminal
insertion is now exercised through rung 32, but one case does not establish
nearest-only attachment as a general policy. The generated parent still needs
two first-branch vias where the historical parent needs none; terminal order,
first-branch alternatives, and bounded fallback to more attachment sources
remain open experiments. Failure kind is still encoded as text even though its
expansions are structured and included in aggregate work. The six-way
conflict-action search, component pressure, and oversized-board continuation
remain separate experimental producers rather than hidden behavior.

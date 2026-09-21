# Blinded board-analysis laboratory

Experiment started 2026-09-07. The question is which evidence presentation
helps a fresh GPT-6-astra agent discover and specify valid board improvements.
This measures diagnostic and repair capability, not autonomous PCB completion.

## Current findings

- Compact JSON and combined views solved all five harder synthetic positives
  initially. Images alone solved three; two exact local queries and two checks
  recovered both failures. A failed image trial was therefore repairable by a
  better interface.
- Native ECC83 trials found improvements beyond the planted defect with both
  JSON and interactive views. Neither representation won consistently on
  quality and time.
- Dense C3 exposed a larger opportunity through connectivity graphs. The first
  graph-based attempt needed native feedback to fix overlapping track-object
  boundaries. A fresh agent using a factual graph and explicit materializer
  passed on its first attempt, saving 731.014285 objective units in 189 seconds.
- Exact acceptance, physical-union scoring, and known-good negative controls
  matter: a visually plausible shortcut failed real pad/hole rules, and simply
  deduplicating stored tracks correctly earns zero gain.

The best interface found so far combines overview/crops, exact object queries,
connectivity, named clearance witnesses, explicit edit materialization, and
native feedback. This is a development conclusion from small trials, not a
statistical ranking. Ground-return quality and production suitability are
outside the current length-plus-via objective.

## Pilot 1: six simple boards

Three fresh-context subagents received the same action objective, with either
compact geometry JSON, aligned two-layer images, or both. They were explicitly
denied the generator, answer keys, other agents' results, and validation feedback.
Their filesystem permissions were shared: blinding was procedural, not enforced.
They did not receive a defect count. Initial proposals were scored only after
submission through the existing Rust semantic geometry/electrical validator.

Four boards contained known improvements: a redundant two-via excursion, an
unobstructed detour, a via excursion replaceable by a short obstacle dogleg, and
a redundant excursion among seven unrelated routes. Two controls were a straight
route and a necessary excursion across a board-spanning top-layer wall.
The via penalty was 2 mm; physical widths and clearance were fixed.

| Presentation | Valid improvements / 4 | Invalid proposals | Correct control abstentions / 2 |
| --- | ---: | ---: | ---: |
| JSON | 4 | 0 | 2 |
| Images | 4 | 0 | 2 |
| JSON + images | 4 | 0 | 2 |
| Deterministic chord/single-layer rewrite | 3 | See below | 2 |

The deterministic baseline exact-tests its trial edits and selects only valid
improvements; its rejected internal probes are not comparable to unvalidated
agent submissions. It cannot construct the dogleg.

JSON and combined agents independently selected the same dogleg, saving
3.820643 mm-equivalent. The image agent selected a slightly more conservative
dogleg, saving 3.794401. Other improvements were identical: 4, 16, and 4.
Images alone therefore suffice for these coarse, integer-coordinate cases.
This is one trial per presentation and is too easy to rank approaches.

## Observable difficulty and resulting tools

- All three agents requested a local clearance/obstacle query for the dogleg.
- The image agent identified exact endpoints, pad layer permissions, and
  obstacle edges as missing evidence. Its integer-axis estimates happened to
  preserve endpoints in this pilot; this cannot be expected on arbitrary boards.
- The JSON agent manually checked layer-specific context in the eight-net case.
- The combined agent used coordinates from JSON and images for spatial context.

Agent observations and tool counts were self-reported, not hidden reasoning
traces. Wall time was approximately two to three minutes per agent but was not
instrumented; no speed or token-efficiency ranking is supported.

The inspection tool now supports exact anchors, layer-transition summaries,
selected-net crops, and a planar segment clearance probe with named blockers
and margins. The action interface also accepts `"start"`/`"end"` references,
so identifying an edit need not require recovering exact endpoints from pixels.
These are explicit coordinate references, not a hidden repair algorithm.

## Pilot 2: narrower margins, more context, and joint edits

Six fresh cases introduce fractional rotated coordinates, a 0.960 mm physical
passage, a blocked 0.820 mm lookalike, foreign-via interference, simultaneous
two-route repair, and 24 isolated corridors with shuffled object IDs.
The required trace-plus-clearance envelope is 0.900 mm. The same three
presentations were assigned to fresh agents. Every arm could use exact endpoint
references, avoiding a trivial penalty for reading fractional coordinates from
pixels. No initial agent received validation feedback.

| Presentation | Valid improvements / 5 | Invalid proposals | Control abstentions / 1 |
| --- | ---: | ---: | ---: |
| JSON | 5 | 0 | 1 |
| Images | 3 | 2 | 1 |
| JSON + images | 5 | 0 | 1 |
| Simple deterministic baseline | 2 | Internal rejected probes | 1 |

The agents discovered a better joint repair for h38 than the stored known
repair: straighten R1 on top while removing R2's interfering layer excursion.
This saves 5.007270 rather than about 1.004 mm-equivalent. Exact validation
accepts this alternative; recovering the author's mutation is not required.

The image agent's h17 estimate used y=15.55. Exact validation found one physical
violation: the valid centerline interval is only y=15.583..15.643. Its h63 edit
named nonexistent R5 instead of R51 and incorrectly inferred that the compressed
lane lacked an obstacle. The real top obstacle is only 0.4 mm high, poorly
resolved at whole-board scale. These were respectively a coordinate precision
failure and a misleading spatial impression plus object-identification failure.

UTC intervals reported by agents were 112 s JSON, 111 s combined, and 199 s
images. These single runs have differing tool activity and shared-host execution;
they are descriptive timings, not a statistically established speed ranking.

### Recovery instead of elimination

The image agent received only its validation outcomes (one physical finding,
and an unknown route ID), then access to the documented local tools. Two
region queries exposed exact obstacles and object IDs. It revised h17's
coordinate and changed h63's diagnosis to a short obstacle bypass.
Two full exact checks admitted both repairs with zero findings.
Thus the same agent recovered to five valid improvements without receiving
an answer key or a parent-selected route/repair. Recovery is a within-episode
result, not an independent fresh-agent comparison.

The next suite rotates, reflects, translates, and relabels these mechanisms
under seed 731904. This checks transfer of the inspection workflow, not unseen
mechanism families. A real KiCad board mutation is being prepared separately.

## Reproduction and retained evidence

Code and protocol: `benchmarks/analysis-lab/`.

```
python3 benchmarks/analysis-lab/generate.py generate
python3 benchmarks/analysis-lab/score.py baseline --output build/analysis-lab/results/baseline.json
python3 benchmarks/analysis-lab/score.py score --answers build/analysis-lab/results/pilot-json.json --output build/analysis-lab/results/pilot-json-scored.json
python3 -m unittest discover -s benchmarks/analysis-lab -p test_inspection.py
```

Public boards and first images: `build/analysis-lab/public/`.
Private known repairs and exact input/candidate artifacts:
`build/analysis-lab/private/`.
Submitted proposals and per-case scores: `build/analysis-lab/results/`.

This initial corpus supports only fixed two-terminal routes, two layers,
circular pads, rectangular keepouts, and no shared copper. Native KiCad,
planes, differential constraints, and placement changes are outside its scope.
More complex fixtures must extend the geometry/measurement contract before
they can earn stronger claims.

### Fresh transformed transfer

A fresh interactive agent inspected six transformed/relabelled boards using
images plus 18 inspection commands and no full repair checks. All five proposed
improvements exact-pass; it abstains on the blocked control. Recorded interval:
11:55:54–11:57:58 UTC (124 s). Six commands were anchor queries, six net queries,
and six segment probes. This does not establish a speed advantage over full
JSON; numerical context remains extremely effective on these small cases.

The query log exposes an interface weakness: `net` returns *all* foreign context,
so its dense-case query could select the wrong net yet still discover the target
elsewhere in the returned data. Successful repair therefore does not by itself
prove efficient spatial localization. A compact route index and stricter local
queries should be measured separately before claiming reduced evidence volume.

## Pilot 3: native ECC83 transfer

An existing working ECC83 board was copied and one straight segment received a
collinear two-via layer excursion. Original/degraded/restored all have zero
native errors and opens, two identical pre-existing silkscreen warnings, and
six identical pre-existing schematic-parity items. Native files and configured
KiCad 10.0.6 DRC govern this trial; synthetic geometry does not admit repairs.

The baseline physical union length is 254.670139 mm. A metric audit found
0.086 mm of duplicate collinear storage; integer-coordinate line grouping and
interval union now exclude it while retaining `stored_length_mm` separately.
The original had 10 vias; the mutated board has 12. Restoring the author's
segment saves exactly four objective units.

Both fresh agents found much larger improvements. They independently noticed
that through-hole pads support copper on both layers, allowing existing vias
to be removed by changing how routes attach to pads. Both initial submissions
pass native DRC with zero opens and no new findings.

| Presentation | Physical length | Vias | Objective improvement | Recorded elapsed |
| --- | ---: | ---: | ---: | ---: |
| Mutated input | 254.670139 mm | 12 | — | — |
| JSON alone | 241.062112 mm | 0 | 37.608026 mm | ~277 s |
| Overview + inspection tools | 246.727000 mm | 0 | 31.943139 mm | 214 s |

The interactive agent used seven inspection commands and two image views.
The numerical agent found a shorter result; this one-board trial does not
establish a presentation winner. Both went beyond the injected mutation and
removed the ten pre-existing vias as well. The public metric correction was
communicated uniformly during the trial; geometry and rule inputs did not change.

Native source, public geometry and mutation proofs live under
`build/analysis-lab/native/`. Agent proposals, logs, repaired boards and DRC
reports live under `build/analysis-lab/results/native-*-evaluation/`.
Evaluator verifies board/project hashes, requires a fresh output directory,
compares full native finding multisets, and rejects a disconnected-deletion
negative even though its raw objective decreases.

## Pilot 4: dense native ESP32-C3

The fully connected C3 baseline has 815 tracks, 196 vias, 190 pads (including
54 roundrects) and zero zones. The mutated input has 817 tracks/198 vias.
Original/degraded/restored native checks preserve 11 warnings and zero
errors, opens, or parity findings. Effective KiCad default rules are exported
explicitly; the original empty project file remains byte-identical.

| Arm | First submission | Accepted objective improvement | Initial elapsed |
| --- | --- | ---: | ---: |
| JSON | Native pass | 51.090634 | ~353 s |
| Overview + local tools | Native pass | 76.834365 | ~347 s |
| JSON + overview, own graph calculations | Rejected: 14 new dangling-track warnings | 0 initially | ~209 s |
| Same graph agent after mapped native feedback | Native pass on first recovery check | 701.088401 | ~80 s recovery |

JSON shortened two signal routes and removed 11 vias through compatible local
edits. The interactive agent used five queries and two image views to replace
long GND perimeter branches and remove the planted pair. Both independently
found both injected vias, alongside improvements that predated the mutation.

The graph agent found 51 cycles in a conservative graph of GND's physical
centerlines. Its proposed tree preserved every pad-center connection and
removed 569.088401 mm physical copper and 66 vias. Initial native connectivity
passed, but keeping overlapping original track objects produced 14 *new*
dangling-end warnings. The strict no-new-findings contract correctly rejected it.

Mapped DRC findings and exact candidate geometry localized the representation
problem. The agent kept the identical selected copper union, replaced all
original GND track objects with non-overlapping atomic segments at explicit
junctions, and passed native DRC on its first recovery check. The final board
has 1989.597721 mm physical copper and 132 vias. This is a substantial gain
under the declared objective; ground-return quality is not measured and this
result is not automatically a production-layout recommendation.

This experiment motivates two distinct tools: a factual overlap-aware
connectivity graph, and a materializer that translates chosen physical edges
into native objects without introducing representation defects. Neither tool
needs to choose the optimization policy on the agent's behalf.

All original native inputs remain unchanged. Successful and rejected proposals,
analysis scripts, candidate boards, and native reports are retained under
`build/analysis-lab/results/dense-*`. The known mutation is only a four-unit
improvement; larger valid alternatives receive credit even if they do not
remove that specific pair.

## Emerging inspection interface

The evidence currently supports a combination of representations rather than a
single universal board view:

- aligned images for spatial discovery and global context;
- exact local coordinates, pad shapes/layers, and named object references;
- physical connectivity graphs for overlap, branching, and cycle reasoning;
- cheap named copper-clearance witnesses for proposed tracks;
- transactional native feedback mapped back to inspectable objects;
- a materializer that preserves the intended physical graph while normalizing
  native track-object boundaries.

The new native copper probe checks straight proposed tracks against rotated
circle/rect/oval/roundrect pads, foreign tracks and through vias after declared
simultaneous removals. On the accepted dense JSON proposal it independently
reproduces the 0.075 mm excess clearance (0.275 actual versus 0.2 required).
It deliberately does not certify connectivity, holes, board edges, dangling
objects or all manufacturing rules; native admission remains separate.

A rejected direct signal shortcut provides a negative calibration: the probe
names pad P32 and track T336 as blockers, and native DRC independently reports
clearance/short/hole/mask violations at those objects. Thus the probe is useful
for answering a specific geometric question without pretending to replace DRC.

The reusable `native_graph.py` now exposes atomic physical edges, pad terminals,
plated-pad bridges, via groups, and their source-object coverage. It performs no
optimization. Its materializer accepts explicit edge selections and preserves
the selected union while emitting non-overlapping track objects. The graph
deliberately omits finite-width contacts away from known centers/endpoints;
coverage and component counts expose that limitation.

An all-edges normalization control on C3 passed native checks with exactly the
same physical length, 198 vias, and 11 warnings. Stored segment length decreased
from 3025.385653 to 2614.921404 mm, but this earns zero score: storage duplication
is not physical copper. Seventeen focused unit tests cover inspection, union
metrics, pad-shape probes, graph construction, and materialization.

### Fresh agent with reusable graph tools

A fresh agent received the same C3 public board plus the graph, materializer,
images, ordinary inspection queries, and copper probe. It received no prior
solution or native validation before submission. It chose its own policy:
compare a pruned spanning tree with 53 terminal-root attachment trees, then
refine branches. This is model-authored search on factual tool output.

The first submission passed native checks, preserving the same 11 warnings
with zero errors, opens, parity findings, or added findings. It removes 72 vias
and 587.014285 mm physical copper: objective gain **731.014285**, resulting in
1971.671837 mm copper and 126 vias. The log reports 188.7 seconds and two clear
copper probes. Scripts, explicit selection, materialized proposal, log, and
native proof are retained under `build/analysis-lab/results/dense-graph-tools*`.

This addresses the earlier graph agent's serialization failure without giving
the new agent an optimization algorithm. It also left enough time for a
stronger search policy. The result is a fresh-agent replication on a reused
development board, not an independent-board generalization or a statistically
established speedup. The same unmeasured ground-return limitation applies.

## Next curriculum: moving necessary geometry

Existing-edge graph selection cannot discover a new geometric shortcut. The
next cases therefore require moving a necessary via or rerouting around
layer-specific barriers, alongside controls where a layer transition remains
necessary. This follows the useful experimental pattern of choosing a local
route, rerouting it, measuring the result, and retaining only accepted gains.
Freerouting documents this optimizer pattern in its
[architecture](https://github.com/freerouting/freerouting/blob/master/docs/architecture.md)
and exposes obstacle highlighting, pull-tight regions, and pin-exit constraints
in its [routing options](https://freerouting.org/freerouting/manual/routing-options).
These are inspiration for benchmark mechanisms and queries, not evidence that
our current tools or agents implement those algorithms.

The first relocation extension has four cases: three improvements that retain
all necessary vias, and a control at the Euclidean-distance plus mandatory-via
lower bound. All baselines and positive witnesses pass the exact validator;
six deliberately invalid alternatives fail it. JSON alone and images with
queries each found **3/3 globally lower-bound improvements**, made no invalid
proposal, and abstained on the control. Recorded elapsed was 86 and 113 seconds,
respectively. Interactive work used eight geometry queries and four planar
probes, plus manual via and simultaneous-edit checks.

The cases extend mechanism coverage but still do not separate strong approaches:
clearance was generous. A new `probe-via` query therefore prepares the next,
narrower-window tests, and both synthetic probes can use explicit simultaneous
proposal context. This answers the agents' need to assess relocated vias and
avoid false blockers from foreign routes that are also being replaced.

## Native residual improvements and pad contacts

An opt-in graph extension recognizes existing nodes strictly inside undrilled
SMD pad copper, adding fixed zero-cost contact edges without adding geometry.
It supports rotated/offset circle, rect, oval, and roundrect pads, while excluding
boundary-only contacts and drilled pads. Default graph JSON/hash remains exactly
unchanged. On the 42 routed C3 nets, modeled terminal connectivity rises from
13/42 to 32/42; pad coverage rises from 83/144 to 134/144. These are graph coverage
counts, not native electrical defects in the source. An all-edge N1 native
control preserves metrics and findings. Twenty-six focused tests now pass.

The strongest accepted graph result becomes residual case n41. Its public
baseline has 1971.671837 mm trace-centerline union, 126 vias, and the same native
findings. A privately checked remaining geometric improvement saves 30.422938
objective units across four non-ground nets. Constructing that witness also
exposed an invalid composition: a relay shortcut depended on ground movement
omitted from the combined proposal. Native admission rejected the clearance,
short, and hole violations; excluding that dependent edit produced the valid
witness. Thus previous acceptance does not make a partial repair safely reusable.

Pad contacts also expose a scoring limitation: the objective counts the union
of track centerlines, including parts inside fixed pads. Removing such parts
can reduce reported length without changing exposed copper or manufacturing
quality. These gains should be reported separately from useful route shortening;
the current trial's objective stays fixed, and no production claim follows.

The fresh residual agent passed native admission on its first submission,
reducing track-centerline union by **46.225237 mm** to 1925.446599 mm, with all
126 vias retained and no new findings. It combined roughly 13 mm of modeled
pad-interior reductions with 42 straight shortcuts along unbranched same-layer
chains. That approximate breakdown is agent-reported, not an independently
scored exposed-copper measure. Elapsed time was 296 seconds: about 80 on graphs,
134 on chord generation/probes, and 82 on combining, reviewing, and logging.

The agent made 500 individual chord probes plus three combined probes and
requested a batch probe and factual unbranched-chain inventory. A new native
batch interface checks explicit candidates independently without generating,
ranking, or selecting them. On 64 saved candidate inputs, full assessments
match separate invocations exactly: **3.7848 s separate versus 0.3288 s batched**
on one local measurement. This is an approximately 11.5x tool-call speedup,
not a measured 11.5x improvement in agent analysis time. Calibration inputs and
results are retained as `probe-batch-calibration*.json`. Twenty-eight tests pass.

The factual chain inventory partitions every atomic track edge exactly once,
with explicit pad/via/branch/width boundaries and closed-cycle handling. On n41
it lists 261 default chains or 301 with pad contacts, preserving the complete
1971.671837 mm path-length total. Twenty-one partial source-coverage entries
make the risk of blindly deleting original track IDs visible.

A fresh agent with chains and batch probes passed native admission on its first
submission, saving **60.579327 mm** (1911.092510 mm remaining, 126 vias unchanged,
no new findings). It tested 125 whole-chain chords and 1620 partial-chain chords,
selected 95 compatible partial chords, and combined them with roughly 13.04 mm
of graph pruning. It excluded incomplete-coverage nets. Total elapsed was 329 s
versus 296 s for the earlier residual agent: the evidence supports greater
search throughput and a stronger result in this trial, not faster total time.
A repeated full-versus-lean batch used the same 1620 candidates and is not
counted as additional distinct search. All diagnostic processes completed.

## Narrow through-via windows

Three new cases isolate via clearance from planar trace clearance. v09 has a
0.060 mm feasible via-center window where traces have 0.460 mm. v26 has a
tempting planar-safe shortcut with no local through-via position; its known
repair routes around a barrier end. v58 rotates/translates the narrow-window
mechanism into 13 routes, with 12 necessary-via optimal lookalikes. The first
and last known repairs attain lower bounds; v26 has no optimality claim.

All baselines and positive witnesses pass exact validation. Three negative
proposals fail only physical checks. Private diagnostic counterfactuals show
their planar geometry passes when the via envelope is shrunk to trace size;
this altered rule is never public or used to accept benchmark proposals.

Initial images-only submissions were rejected on all three cases: physical
finding counts 1/2/2, with three additional electrical findings on v26. That
case exposed an interface ambiguity: the image did not label route start/end
order, and the agent applied bottom/top layers to start/end references whose
actual order was reversed. Treat this as presentation metadata missing from
the image, not a failure to understand layer connectivity.

Images plus exact queries and explicit via probes passed **3/3** initially.
The v09/v58 vias have 0.03 mm clearance margin; v26 uses a corner-clearing path
with only 0.004599 mm via margin and saves 1.701587, exceeding the stored witness.
The visual-only agent is being recovered in stages, beginning with endpoint
metadata and crops, to distinguish improved image presentation from numerical
clearance-query assistance.

The fresh JSON arm also passed 3/3, with a slightly more conservative v26 gain
of 1.691837. Approximate initial elapsed times were 91 s JSON, 107 s interactive,
and 151 s images-only. These are single runs with different inspection work.

The same images-only agent then recovered **3/3** with endpoint anchors and
enlarged renderings alone, in 111 s. It used no numerical obstacle query, via
probe, or exact acceptance check during recovery. It corrected v26's endpoint
layer ordering and estimated narrow-window centers from enlarged overlays;
v26's accepted gain was a more conservative 1.474704. This demonstrates that
resolution and metadata were sufficient to recover these failures, not that
only a clearance oracle could solve them. Recovery also includes error counts
and additional analysis time, so it is not a clean timing comparison.

Future synthetic renderings label pad endpoints S/E to remove the observed
ordering ambiguity. Earlier trial images remain preserved unchanged. A separate
via-center clearance-space view is being developed to expose both-layer
constraints directly without choosing a repair.

`via_space.py` now renders explicit excluded center regions using circular
Minkowski corners/caps for both layers, with exact crop ticks and named blockers.
Optional point evidence is separate from the map-only experimental arm. Three
geometry/raster/transaction tests and explicit public-v26 sample checks agree
with the via probe; the complete laboratory now has 41 passing tests. The map
does not certify full routes, connectivity, holes, or board edges.

A fresh visual agent using ordinary images, anchors, and via-space maps passed
3/3 on its first submission in 136 s. It was prohibited from numerical point
queries, raw board JSON, and the renderer's JSON evidence. New via coordinates
were read from image axes. The v26 gain was 1.573999, with the other two reaching
their lower bounds. This is another successful presentation, not a speed winner:
JSON and ordinary images with exact queries were quicker in their single runs.

## Practical rule and connectivity coverage

The project's existing [Olimex preflight](olimex-c3-rule-preflight.md) identifies
a real adapter gap: local mounting-hole pad clearance was lost in the uniform
net-class representation. Read-only inspection confirms native own-clearance
values of 1.85 mm for mounting-hole pads, 1.016 mm for fiducials, and 0.254 mm
for some connector pads, compared with a 0.127 mm default. The retained Olimex
router result has two opens and existing errors, so it is an audit input,
not another clean benchmark board or accepted solve.

The exporter now records resolved own clearance separately for every pad/layer,
and the probe honors that value with the proposed-track rule and board minimum.
The read-only audit detects nine existing mounting-hole blockers missed by a
packet stripped back to the old 0.127 mm netclass fallback. Gap measurements
match native reports within 0.00011 mm. These are native `hole_clearance`
findings on NPTH pads whose 3.3 mm circular pad outline equals the drill outline;
this equivalence is specific and does not establish general hole-rule support.
Eleven targeted export/rule/probe tests pass. Audit evidence lives under
`build/analysis-lab/olimex-rule-audit/`.

For this audit only, the exporter recognizes zero-delta trapezoid tags as exact
rectangles while retaining their source tags, and records non-copper rule
areas that allow tracks/vias as separate metadata. The Olimex input has ten
such rectangular tags and one area that only forbids zone fills. Native objects,
board/project hashes, and earlier public benchmark packets remain unchanged.
Actual copper zones, routing-restrictive areas, and nonzero-delta trapezoids
remain unsupported. General holes and custom pair rules still require native
DRC; the inspection tool has not become a complete rule engine.

Native direct-contact evidence offers another improvement in graph coverage.
`GetConnectedPads(track)` gives directly touching pads; requiring the native
pad hit-test to contain an actual track endpoint provides a fixed-pad contact
witness. The completed read-only C3 extractor raises modeled connected routed nets from
13/42 to 42/42, compared with 32/42 for the SMD-only geometric inference. This
does not claim to model every interior crossing or finite-width track contact.

The packet contains 337 records, producing 170 fixed contact edges and covering
144/144 routed pads. All 172 prior default/SMD-inference graph hashes remain
unchanged. N31 and N18 all-edge normalization controls both pass native admission
with unchanged physical length, via count, and cost, and no added findings.
These are normalization controls, not additional agent trials or evidence that
arbitrary graph selections are safe. Native pad hit-tests do not subtract drill
voids. The final analysis-lab unit suite passes 55 tests.

Exploration stopped at the user's request. The next step is integration into
the main program, following the [program review](../reviews/2026-09-07-program-review.md).

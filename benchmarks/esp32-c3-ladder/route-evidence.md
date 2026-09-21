# ESP32-C3 progressive route evidence

This file records how supplemental copper entered the declaration. KiCad ERC,
DRC, schematic parity, and selected-net completion remain the acceptance
authority; A* success alone is never treated as a finished rung.

| Connection | Context | Producer | Resolution | Cost | Expansions | Outcome |
| --- | ---: | --- | ---: | ---: | ---: | --- |
| `T_PULL_PROBE` | 15 | hand-built two-via baseline | — | — | — | Prefixes through 17 pass |
| `T_BUS_RELAY1` | 18 | hand-built four-via baseline | — | — | — | Rung 18 passed, but XDATA later exposed three conflicts |
| `T_PROBE_XDATA_1` | 22 | `testing-esp32-duts-astar-v2` | 0.25 mm | 11,987 | 29,185 | Rung 22 passed, but XDATA later crossed it |
| `T_BUS_RELAY1` | 23 | `testing-esp32-duts-astar-v2` | 0.10 mm | 30,517 | 389,419 | Future-aware replacement passes prefixes 18–31 |
| `T_PROBE_XDATA_1` | 23 | `testing-esp32-duts-astar-v2` | 0.10 mm | 26,464 | 155,808 | Future-aware replacement passes prefixes 22–31 |
| `D_BUS_DHT` | 25 | `testing-esp32-duts-astar-v2` | 0.25 mm | 17,737 | 10,479 | Prefixes 24–31 pass |
| `D_BUS_US_TRIG` | 29 | `testing-esp32-duts-astar-v2` | 0.25 mm | 8,770 | 11,853 | Prefixes 28–31 pass |
| `D_BUS_US_ECHO` | 31 | `testing-esp32-duts-astar-v2` | 0.25 mm | 8,626 | 13,259 | Prefixes 30–31 pass |
| `T_EN` | 37 | rooted-star over `testing-esp32-duts-astar-v2` | 0.25 mm | 31,788 | 44,183 | Four pads/three branches; prefixes 32–42 pass |
| `D_BOOT` | 36 | `testing-esp32-duts-astar-v2` | 0.25 mm | 6,421 | 9,645 | Prefixes 35–42 pass |
| `T_SERVICE_RX` | 37 | `testing-esp32-duts-astar-v2` | 0.25 mm | 11,450 | 6,220 | Prefixes 36–42 pass |
| `D_SERVICE_RX` | 39 | `testing-esp32-duts-astar-v2` | 0.25 mm | 14,999 | 37,422 | Prefixes 38–42 pass |
| `D_SERVICE_TX` | 40 | `testing-esp32-duts-astar-v2` | 0.25 mm | 14,917 | 31,080 | Prefixes 39–42 pass |
| `3V3_DUT` | 41 | rooted-star over `testing-esp32-duts-astar-v2` | 0.25 mm | 195,995 | 744,282 | 13 pads; persisted 61-segment/12-via replacement exact-passes |
| `GND` attempt 1 | 42 | rooted-star over `testing-esp32-duts-astar-v2` | 0.25 mm | 885,461 | 2,500,137 | Reduced 277 DRC findings to two same-net via-hole violations |
| `GND` attempt 2 | 42 | rooted-star with inter-branch drill reservation | 0.25 mm | 1,232,242 | 3,662,940 | 41 physical pad centers; persisted 412-segment/128-via replacement exact-passes |

The two rejected prefix-only routes are retained as evidence for a planning
problem, not as evidence against grid A*: a route can be locally valid yet
consume a corridor needed by a later declared connection. The replacements
were searched with the next relevant copper already active. That is a small
baseline for later alternative/topology search, where the system should keep
several valid decisions instead of committing irreversibly to the first one.

The 0.10 mm repairs and multi-terminal rooted stars contain short staircase
segments. They are valid but not good final geometry. A bounded line-of-sight
pass or the push/pull engine should shorten them, with every proposed
simplification rechecked by KiCad.

Sequences 019–022 exposed an error in the quality metric. It summed stored
track objects and deduplicated only complete byte-identical segments, so it
counted partially overlapping same-net trunks more than once. The original
candidate contains 238.146317 mm of canonical physical copper but 294.381599
mm of stored track-object length; 56.235281 mm is overlap representation debt.
All earlier “physical copper” labels in the corresponding JSON records are
historical implementation output, not the current acceptance measure.

With the corrected metric, greedy and retained-visibility shortening still
produce the same proposal after 201 visibility tests, but it is a negative
result: stored length falls to 238.414449 mm while canonical physical copper
grows by 0.268131 mm. Removing branch vertices destroyed shared structure.
Both processors now roll back as `no_improvement`. Sequence 023 retains the raw
shortener board so this is visible rather than silently erased.

The branch-0 projected-tension mechanism remains positive when rerun from the
original candidate. It moves the selected interior points by at most 0.666849
mm and improves canonical copper from 238.146317 mm to 237.552374 mm. The
2.0 mm uniform-grid broad phase retains 22 of 879 layer-relevant obstacle pairs
and spends 19,712 rather than 787,584 projection rows while producing
byte-identical exhaustive copper. The earlier no-guard and undersized-envelope
controls remain valid safety evidence, but their old length numbers use the
superseded track-object metric.

The sequence-022 exact-shared-point conclusion is retracted. Exact coincident
vertices do not capture collinear shared subsegments with different endpoints;
under the corrected metric its current raw proposal grows physical copper from
238.146317 mm to 274.459420 mm and rolls back. A zero-motion route-graph control
finds eight same-layer physical contacts, inserts seven vertices, and leaves
physical length exactly unchanged. One contact joins a fixed terminal to a
movable branch interior, so the generic compiler anchors the shared particle
rather than rejecting or detaching it.

The context-aware graph six-branch run finds one additional contact with the
unselected same-net tree and fixes three context nodes. It improves canonical
copper to 228.464953 mm, a real 9.681364 mm reduction, and deduplicates 16
tension edges. The local
broad phase retains 272 of 5,274 layer-relevant obstacle pairs and spends
169,728 rather than 2,812,800 projection rows (16.6x less), with byte-identical
candidate and proposal files relative to exhaustive mode. Rungs 41 and 42 pass
the native gates with zero ERC, DRC, parity, or selected-net connectivity
findings. Evidence is in
[`experiments/3v3-route-graph-projected-tension.json`](experiments/3v3-route-graph-projected-tension.json),
and all six source/rejected/control/accepted states have front/back renders in
sequence 023 under `build/progress/`; sequence 030 stage 04 renders the
context-aware result. This sharpens the particle lesson: more
samples can improve resolution, but one physical node or edge must not become
multiple independently movable particles or objectives.

Sequence 031 exercises the same rule across actual fixed vias. Selecting two
branches together lets their shared post-via junction move and improves the
canonical tree from 236.745909 mm to 232.762704 mm; selecting only one branch
reaches 235.943377 mm because that junction becomes fixed context. The matched
independent-particle proposal fans coincident trunks apart and regresses to
292.572328 mm, so it rolls back despite passing KiCad connectivity and DRC.
The uniform-grid and exhaustive shared-graph candidates are byte-identical;
the local run spends 115,200 rather than 1,298,432 projection rows. Evidence
is in
[`experiments/rung41-shared-via-motion.json`](experiments/rung41-shared-via-motion.json),
with every source/control/proposal rendered front/back in sequence 031.

Sequence 032 removes the manual bundle choice from that experiment. With
branch 1 as its only seed, one `nearest_shared_junction` hop selects branches
`[1, 2]` after 830 bounded segment-contact tests. The relaxed candidate,
proposal, and applied board are byte-identical to the manual control. Limiting
the selector to one branch rejects before engine compilation, emits no raw
proposal, and keeps the source byte-identically. Evidence is in
[`experiments/rung42-automatic-branch-neighborhood.json`](experiments/rung42-automatic-branch-neighborhood.json),
with all four board states rendered front/back in sequence 032.

Sequence 033 adds discrete via lifecycle actions before continuous motion. On
branch 8, merging away the selected bottom-layer excursion yields a tempting
234.889343 mm/two-via reduction but crosses the occupied front routing field
and produces 19 DRC findings; merging toward the back produces 28. Of twelve
relocation sites, only two native-complete positions lie north of the source.
The selected 1 mm move improves 0.269941 mm. Fixed-via tension then improves
either seed by the same 0.475866 mm, so relocation plus tension reaches
236.000102 mm versus 236.270043 mm for tension alone. Evidence is in
[`experiments/rung42-via-topology-actions.json`](experiments/rung42-via-topology-actions.json),
and sequence 033 renders the source, all fourteen actions, and both controls
front/back.

Sequence 034 replaces the twelve blind relocation sites with a bounded
analytic-feasible radial frontier on the same via and radius. The preliminary
ranking wasted all four native slots on micrometre variants of one boundary;
those boards are retained rather than hidden. Score-ordered 0.1 mm geometric
suppression leaves 28 distinct improving points before the four-candidate cap.
The generator performs 646 analytic geometry probes, then native-gates four
relocations plus the two unchanged removal controls. All four relocations pass,
and the selected site reaches 236.426303 mm raw and 235.950438 mm after the
same tension pass. That is 0.049665 mm better than sequence 033 at both stages,
while the continuous improvement remains exactly 0.475866 mm. The sampled
frontier therefore improves native-gate efficiency on this fixture without
establishing a new relaxation basin or a complete continuous search. Evidence
is in
[`experiments/rung42-via-feasible-frontier.json`](experiments/rung42-via-feasible-frontier.json),
with all preliminary and corrected states rendered front/back in sequence 034.

Sequence 035 tests removal as a transaction rather than judging its illegal
direct chord. After atomically merging away a target via, a bounded local
single-layer A* pass repairs the new maximal same-layer run before canonical
materialization and the full native gate. On branch 8, six searches in a 4 mm
window and six in a 12 mm window all report no path; the finest front searches
expand 81,794 and 116,122 states respectively. This rules out the small window
as the main cause and preserves that excursion under the current zero-new-via
action semantics.

The branch-9 private excursion is the positive real-board control. Its direct
front/back merges produce 11/24 DRC findings, but front-layer local repairs at
0.5, 0.25, and 0.125 mm all native-pass and reduce the physical via count from
12 to 10. The 0.25 mm candidate is selected at 232.730760 mm versus the
236.745909 mm source; it uses one search and 18,088 expansions. The 0.125 mm
grid spends 77,085 expansions and finds a 0.116402 mm shorter local path, yet
its canonical whole-tree copper is 236.171339 mm because it reuses the
existing tree less effectively. This is direct evidence that local A* cost
cannot be the action portfolio's multi-terminal objective. Full evidence is
in
[`experiments/rung42-via-removal-local-reroute.json`](experiments/rung42-via-removal-local-reroute.json),
and sequence 035 renders every materialized board plus explicitly named
unchanged-source pairs for no-path actions.

Sequence 036 replaces those unchanged no-path frames with actual search
diagnostics. Portfolio schema v4 retains typed failure kind, affected branch,
endpoints, window/grid geometry, obstacle inflation, losslessly row-run-encoded
blocked cells and blocked frontier, and search/retry work. A topology-only
candidate and paired front/back plan render are written under each unsupported
attempt but remain ineligible for selection. At 0.25 mm, the front search has
10,769 states, 6,092 blocked states, and a 361-cell frontier after 19,126
expansions; the back search has 13,392 states, 9,025 blocked states, and a
293-cell frontier after 10,788 expansions. Refinement increases detail without
opening a corridor. The rendered frontier shows the front search closed by
horizontal/vertical occupied bands and the back search closed against a
diagonal track bundle and pads. Evidence and the representation ablation are
in
[`experiments/rung42-via-reroute-frontier-diagnostics.json`](experiments/rung42-via-reroute-frontier-diagnostics.json),
with all nine front/back stages in sequence 036.

Sequence 037 ports the DUT router's semantic frontier boundary into the KiCad
adapter. Pads remain grouped by footprint reference with the union of their
foreign nets and declared `Router_Movable` state; tracks and vias are grouped
by normalized foreign net. One owner is counted at most once per frontier
cell, even when several primitives overlap. All 2,596 cells across the six
branch-8 searches are attributed. `F.Cu` has the same nine blockers at every
resolution, led by fixed `J4`, `D_EN`, and `D_LOCAL_COMM0`, while movable `C4`
and `R19` remain smaller contributors. `B.Cu` is entirely foreign-net copper,
led consistently by `T_EN` and `GND`; only the low-hit tail changes with grid
resolution. This is boundary pressure, not proof that the ranked owners form a
minimal cut. Full evidence is in
[`experiments/rung42-via-reroute-semantic-blockers.json`](experiments/rung42-via-reroute-semantic-blockers.json),
and sequence 037 repeats all nine paired stages with semantic centroid labels.

Sequence 038 distinguishes frontier pressure from causal sufficiency. Its
bounded breadth-first ablation search removes one semantic obstacle owner,
reruns the local A*, and expands a failed trial using the blockers on that new
frontier. A reduced two-wall regression matters here: `WALL_B` is absent from
the root frontier and appears only after suppressing `WALL_A`; the pair then
opens the route. On real `F.Cu`, all 50 generated one/two-owner trials fail
after 1,063,848 counterfactual expansions. On `B.Cu`, `GND` alone opens a
four-point 44.152543 mm path; four successful pairs containing GND are merely
redundant supersets. The ablations never become candidates and the native-valid
source stays selected. Full evidence is in
[`experiments/rung42-via-reroute-blocker-cuts.json`](experiments/rung42-via-reroute-blocker-cuts.json),
with every original failure and all 73 trials paired front/back in sequence
038.

The first real-board terminal-attachment control is rung 1 rather than the
full board. Projected tension attaches the selected route to fixed `U1` and
movable `R1`, and candidate files now carry revision-checked footprint poses
through direct application and ladder rematerialization. Horizontal-only
motion shifts `R1` by 0.229717 mm and reduces the fresh A* candidate from
19.617588 mm to 18.173818 mm. The complete board passes native KiCad with zero
ERC, DRC, parity, or selected-net connectivity findings. Unconstrained
translation and free rotation do move the body (the latter by 1.767492°), but
both are rejected because `R1` silkscreen crosses the adjacent `R2` reference.
A one-step ablation still fails: the source has effectively no upward
silkscreen margin. This is evidence for adding placement/body constraints, not
evidence against free rotation. Full measurements and all controls are in
[`experiments/rung1-terminal-component-coupling.json`](experiments/rung1-terminal-component-coupling.json),
with front/back sequence-024 images under `build/progress/`.

Sequence 025 adds generic oriented body/body projection and tests the missing
placement rows rather than merely forcing the safe axis. The first prototype
preserved `R1`'s signed courtyard distance to all 32 other footprints. It
native-passed but froze `R1` completely, so that construction was rejected.
A conservative motion-envelope broad phase now retains only footprints that
the configured translation/rotation range could reach. The X/Y-translation
run retains two pairs, moves `R1` by 0.229717 mm, reaches 18.173812 mm, and
native-passes. That is within 0.000007 mm of the horizontal-only control while
removing the hand-selected axis restriction. The free-rotation run retains
three pairs and also native-passes at 18.174384 mm, but rotates by only
0.003058° versus the invalid unconstrained control's 1.767492°. This is a
positive placement-safety result and a negative rotational-freedom result.
Courtyards also omit footprint reference text, so native KiCad remains the
acceptance authority. Measurements and conclusions are in
[`experiments/rung1-placement-clearance.json`](experiments/rung1-placement-clearance.json),
with every source, rejected, overconstrained, accepted, and control board
rendered front/back in sequence 025 under `build/progress/`.

Sequence 026 tests the alternative suggested by that rotational-freedom
failure: move movable documentation rather than forcing `R1` away from it. A
source-checked port of `testing-esp32-duts`' reference-field relaxer stores the
field UUID plus old/new local poses in the route candidate, so direct
application and ladder rematerialization reproduce the same board. Moving
`R2`'s reference east or west makes the unrestricted 1.767492° rotation pass
the complete native KiCad gate. Moving it to the opposite vertical side fails
because it overlaps `R3`, and the fourth inherited action is byte-identical to
the first for the axis-aligned source pose; the evidence therefore reports
three unique actions rather than four.

This recovers capability, not quality. The repaired free route remains
18.218363 mm versus 18.174384 mm for nearby-courtyard free rotation and
18.173818 mm for the horizontal control. The repaired unrestricted translation
is similarly 18.191331 mm versus 18.173812 mm for its nearby-courtyard control.
The result supports keeping reference relocation as a discrete DRC-driven
action, but it does not select it for this rung. Full single-action evidence is
in
[`experiments/rung1-reference-field-relaxation.json`](experiments/rung1-reference-field-relaxation.json),
with all six front/back stages in sequence 026 under `build/progress/`.

Sequence 027 closes the manual-search gap. The coordinator first reconstructs
the invalid free-rotation board and runs a fresh native gate, so a stale caller
DRC cannot choose actions. It then reduces four predecessor phases to three
unique candidates, snapshots and native-gates all three, retains the failed
vertical cascade and both valid sideways moves, and emits the selected
source-checked candidate. East and west have identical 18.218363 mm routes and
3.111270 mm reference displacement, so the final predecessor-phase tie-break
selects east. This automates validity recovery but does not alter the negative
quality conclusion relative to the 18.174384 mm courtyard control. The search
is still reference-specific, sequential, and has no readability objective
beyond clearance and displacement. Evidence is in
[`experiments/rung1-reference-action-portfolio.json`](experiments/rung1-reference-action-portfolio.json),
the complete retained search is under
`build/experiments/027-reference-action-portfolio/`, and all four investigated
action-search boards are rendered front/back in sequence 027. A fifth no-op
control starts with the native-valid sequence-025 courtyard candidate; the
coordinator emits zero repair actions and returns it byte-identically rather
than manufacturing a change.

The visible GND via meshes are quantified in
[`experiments/gnd-via-cluster-diagnosis.json`](experiments/gnd-via-cluster-diagnosis.json).
The candidate has 128 vias; 91 have another via within the 1.0 mm diagnostic
threshold. The ten-via mesh beside the tester module comes from five adjacent
GND-pad branches independently repeating a two-via layer-switch maneuver. All
of that copper is GND and KiCad reports no cross-net violation, so this is a
redundant same-net topology rather than an observed hidden short. A reduced
five-pad shared-tree/canonicalization fixture should precede any attempt to
tune the entire GND net.

Sequence 039 tests the more appropriate GND representation from the DUT
predecessor. A source-preserving transaction replaces all 412 GND tracks with
solid-connected F.Cu/B.Cu zones and invokes KiCad with `--refill-zones` before
acceptance. Zones without stitching leave five unconnected items. Retaining
the existing 128 vias passes; deterministic board-cell thinning also passes at
83, 50, and 32 vias. The tested 26/24/22/18/8/1-via sets leave two to four
unconnected items. The declaration now selects the 8 mm/32-via point, so a
fresh rung 42 has 403 total segments, 100 total vias, two zones, and zero ERC,
DRC, parity, or selected-net connectivity findings. This removes the visible
routed-via meshes but does not establish optimal stitching: the seed pool is
still the old rooted-star route and cell-size changes are order-sensitive.
Full measurements are in
[`experiments/rung42-ground-zone-via-thinning.json`](experiments/rung42-ground-zone-via-thinning.json),
and every passing and failing board is paired front/back in sequence 039.

Sequence 040 closes the immediate foreign-zone integration failure exposed by
the full regression suite. Treating a zone polygon as fixed copper blocks most
of the board, but silently discarding all GND geometry would also be wrong. The
routing model now omits only the yielding fill polygon; GND pads and the 32
retained stitching vias remain ordinary hard obstacles. KiCad then refills and
native-gates every proposal. In a bounded branch-8 `3V3_DUT` portfolio, both
0.5 mm via relocations now pass rather than only one under the old routed GND
tree. The selected second action improves physical copper from 236.745909 mm
to 236.605431 mm. This proves transactional coexistence, not preserved plane
quality: area loss, necks, islands, and return paths are not scored. Evidence
is in
[`experiments/rung42-yielding-zone-via-relocation.json`](experiments/rung42-yielding-zone-via-relocation.json),
with the source and both actions paired front/back in sequence 040.

All 43 phases now pass. This closes the declaration-to-verified-KiCad pipeline
milestone; it does not endorse the rooted-star topology. The next experiments
can compare shared-tree growth, conflict-action search, push/pull shortening,
and oversized-board continuation on smaller discriminating prefixes while the
full board remains a regression.

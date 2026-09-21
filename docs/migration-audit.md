# Predecessor migration audit

The source projects are:

- `/home/flo/work/testing-esp32-duts`
- `/home/flo/programming/layout-trace`

They remain read-only references. This repository imports reviewed source and
fixtures, not their build trees or generated result archives.

## Selection rule

`layout-trace` supplies the durable problem/solution/topology model and the
verification-oriented pipeline spine. `testing-esp32-duts` supplies competing
algorithms and useful system-level workflows. This is not a declaration that
the former always optimizes better. In particular, the DUT raster/A* and
particle approaches remain live experiments that should be tested with the
Rust coordinator, larger search budgets, and common exact gates.

When implementations conflict, the choice is based in this order:

1. semantic correctness and fail-closed validation;
2. ability to compare implementations behind a stable boundary;
3. deterministic work accounting and reproducibility;
4. suitability for data-parallel CPU/GPU execution;
5. measured result quality and resource cost on the same fixtures.

A single worse result is diagnostic evidence, not grounds for deleting an
approach.

## Migration matrix

| Subsystem | Selected basis | Current state | Reason / next port |
| --- | --- | --- | --- |
| Semantic problem and schema | `layout-trace-model` | Ported | Covers layers, pads, movement/rotation constraints, keepouts, two-pin compatibility nets, and multi-terminal nets without solver representation leaking in. |
| Geometry and homotopy primitives | `layout-trace-model` | Ported | Stable route-class vocabulary is needed before comparing routers. |
| Pass composition | `layout-trace` stage contract | Ported as `pcb-pipeline` | Fail-closed invariant certificates and explicit technical debt are stronger than relying on pass order. Runtime transactions and budgets remain to be wired in. |
| Candidate route graph | `layout-trace` | Ported for current grid candidates, shared-tree growth, router-independent adjacent-junction transactions, and the KiCad continuous handoff | Components, explicit terminal/junction/via nodes, split branches, segment layers, stable identities, and partial failed graphs are durable. Search-time tree growth materializes non-parallel same-layer segment contacts in both polylines; it and the bounded post-route transaction rebuild exact incidence rather than inferring connectivity from contact. The KiCad handoff additionally normalizes collinear overlap, T contacts, and crossings among selected same-layer branches, including fixed-terminal/interior mobility. Collinear search-time tree attachment, via contacts, contacts with unselected continuous branches, corridor/topology artifacts, and full pipeline certificates remain. |
| Electrical connectivity gate | `layout-trace` | Ported in `pcb-validate` | Preserves explicit multi-terminal reachability, attachment/layer checks, deterministic findings, and tamper tests. |
| Exact geometric DRC | Reviewed extraction from `layout-trace` semantics | Exhaustive reference ported in `pcb-validate` | Independently checks component pose freedoms/regions/overlap, board containment, pad/keepout clearance, inter-net copper, and vias. Parity tests against the predecessor and richer fabrication rules remain high priority. |
| Raster grid router | `testing-esp32-duts` native Rust A* plus reviewed Toit semantics | Usable hard-reservation, negotiated-congestion, selective-reroute, and shared-copper-tree slices in `pcb-grid-router` and `pcb-routing` | Keeps bounded deterministic search, bend/via/dynamic/history costs, an admissible heading-aware bend lower bound, no diagonal corner cutting, arbitrary layers, bounded shape/segment rasterization, route ordering/priority overrides, stable multi-terminal growth slots, blocked-frontier ownership, coarse-to-fine hard retries, exact-invalid directional-edge learning, fixed-copper local reroute, durable per-pass failure evidence, and exact validation. Shared routing starts at a stable root, dynamically minimizes over current tree vertices and pending terminals, aborts dependent growth after failure, trims through the last exact same-layer contact (including non-parallel segment interiors), emits explicit split/junction graphs, and optionally evaluates bounded connected-terminal alternatives using predecessor via/length/bend ordering. A separate bounded, spatially diverse source portfolio compares retained length, predecessor ordering, or router cost and retains every trial. Collinear/via attachment, whole-tree lookahead, synthetic-branch selective ownership, corridors, waypoints, completion, topology shake, and bus lanes remain. The old 71/83 dual result is retained as historical scaling evidence, not a tuning target. |
| Corridor/visibility router | `layout-trace-routing` extracted kernel | Kernel, candidate adapter, bounded fixed-context repair, and a first native via-family producer ported | The complete corridor, copper-pair, family-portfolio/assignment, fixed-context correction, and combinatorial-regime kernel passes its tests. Per-layer corridor analysis, joint family selection, smallest-conflict-component repair, and opt-in visibility work budgets are selectable CLI paths. V5 adds fail-closed ordered runs, explicit via geometry/clearance certificates, and exhaustive conflicts across every run and via annulus. The native producer samples bounded seed-derived via sites, searches each run, preserves spatial diversity, and exact-passes clear, blocked-midpoint, and interacting two-net controls; the latter selects distinct via positions and is bounded infeasible with one shared site. Clearance handling is an explicit discrete-separation versus repair-obligation policy, preserving the predecessor behavior without sending unsupported multi-run obligations into the continuous engine. An imported portfolio separately gates the file boundary. Incremental context promotion, terminal escape, adaptive two-dimensional via sites, multiple transitions, and corridor viewer overlays remain. |
| Initial placement | Reviewed `layout-trace` policies plus `testing-esp32-duts` canonical batch-evaluation boundary | Ported as `pcb-placement` with a cold native portfolio coordinator | Declared, grid, deterministic random, barycentric, and harmonic-port policies share alternating relational/overlap projection, feasibility validation, configs, work evidence, and free rotation. Placement exclusion conservatively includes pad extents after the dual native rung exposed an ESP32-pad/passive short that nominal bodies missed. The historical harmonic connected-pair floor remains an ablation switch. A bounded portfolio now evaluates each placement using a separate zero-copper ladder and complete native progression; selection uses completion before physical copper/via quality, never ratsnest score alone. On dual prefix 5, harmonic cuts copper 20.58% versus declared while both native-pass, but uses 2.68x routing expansions. Under-anchored harmonic blocks now have explicit declared/grid/bounded-random seed policies. A corrected four-entry control finds one random seed native-complete on attempt 11 with 5.04% fewer route expansions, but 0.995 mm more copper and two extra vias, so the declared fallback remains selected. At prefix 7 the random pose is 3.24% shorter but has four additional vias. A diverse/router-cost shared-tree entry reuses an existing layer change and removes one via from both cold boards without adding copper; historical placement still wins by 1.372 mm. Schema-v4 evidence-directed activation reproduces both unconditional boards byte-for-byte with 5.46%/8.01% fewer route expansions. Archived parents and failure-directed placement composition remain. |
| Discrete repair | Both | Transactional policies in `pcb-coordinator` with independently selectable pressure, propagation, ordering, direction, beam, and topology policies | Blocked grid frontiers retain semantic owners and physical centroids. Maximum-distance propagation and pad-extended collision chains respect placement clearance. Failed-branch endpoints, multi-branch ordering, bidirectional proposals, beam width, neutral-state retention, and selective rip-up are opt-in. Mixed feedback survives cold prefix 14. Prefix 15 proves why semantic success is only a proposal: greedy, mixed, and placement-only beams improve semantic completion but each perturbs native routing into an earlier dead end. Cold-feedback schema v2 retains that complete failed trial, independently regenerates base placement/routes from zero copper, and selects the farther exact result; the fallback PCB is byte-identical to the no-feedback control. Future-connection reachability during native candidate selection, rotated/pad-aware passage inference, longer/nonuniform push chains, synthetic split ownership, rigid movement groups, and negotiated conflict groups remain. |
| Progressive connection insertion | `layout-trace` transaction contract plus current DUT A*, pressure coordinator, and KiCad adapter | Ported through generated KiCad rung 29 with bounded multi-resolution quality selection | Exact-delta checks, immutable parents, bounded portfolios, whole-target certification, exact rollback, hashes, work telemetry, and native admission are present. Transaction schema v2 also records terminal count and removes only portfolio entries whose differing tree-policy fields are inactive for two-terminal connections; the prefix-8 control remains byte-identical while saving 19.91% of portfolio expansions and 31.25% of native gates. A three-resolution chain commits rungs 1–21 locally at 477.768 mm/14 vias versus 553.350 mm/22 vias historically; score-ordered and fail-closed adaptive modes retain explicit work/quality evidence. Rung 22 adds `T_PROBE_XDATA_1`. Shared-tree `XDATA` commits the first four-terminal child at rung 23, improving the rooted-star control from 25.488 mm/four vias/one close pair to 22.405 mm/two well-separated vias. Nearest-only attachment reproduces the eight-source boards with 44,744 rather than 65,812 expansions; a retained diversity ablation regresses to 192,451. Isolated native-complete steps continue through rung 29. `US_TRIG` proves another conservative raster threshold: 0.125/0.10 mm falsely agree on a 65% detour, the historical route passes on the exact generated parent, and 0.05 mm finds a 6.649 mm route 14.84% shorter than historical. Paired evidence covers every state. Evidence-driven refinement, terminal/first-trunk alternatives, opposite routing order, genuine multi-net rip-up, component/continuous fallback, and full topology/action search remain. |
| Continuous push/pull engine | Both | CPU reference connected to durable exact candidates, selected family obligations, and staged width continuation | Particle/field SoA coexists with analytic body pose/inertia and arbitrary body-local terminal attachments. Endpoint-distance and rigid-attachment policies have matched field-load, rotation, and exact-materialization controls. The representation-independent solution carries semantic poses and ordered traces into the common candidate. Same-layer obligations compile to bounded segment/segment rows, semantic rectangles compile to segment/body rows, and generic body/body rows support angular response. Width continuation routes a thin topology with full clearance, splits mixed-layer traces into same-layer runs, represents fixed vias and circular pads as zero-length circular polylines, lowers rectangular pads as oriented bodies, and exact-gates every stage. Per-point/body trust regions retain bounded backtracking evidence. A trace-first/component-fallback processor keeps component motion out of stages trace motion can solve. Dual prefix 19 reaches 24/24 at 5% and exact 25% width, then both processors roll back at 27.5%. Exact rigid compound shapes on moving bodies, relational constraints, movable vias, local contact neighborhoods, remeshing, stochastic movement, calibrated mass semantics, and dynamic broad phase remain. |
| GPU backend | New Rust implementation | CPU reference and flat demand-field diagnostic present; device backend deliberately deferred | Preserve semantics with a CPU reference first; design GPU batches around data and constraint families rather than component types. Prefix-15 timing shows direct local A* is a small minority of wall time and no current attempt approaches its two-million-expansion ceiling. The host GeForce GTX 1650/Vulkan stack is usable outside the sandbox, so device work is testable, but the user has deferred it. First device candidates remain batched placement demand/force fields, contacts/constraint rows, candidate-parallel reachability, and wavefront routing—not merely one irregular priority-queue A* search. Exact native admission stays outside the device loop. |
| KiCad import/export and external checks | Reviewed `testing-esp32-duts` workflow, native Rust materializer, bounded DUT A*, and KiCad CLI | All progressive C3 prefixes 0–42 exact-pass | `pcb-kicad` filters the preserved generated project from a 42-connection ordered declaration, exports the resulting schematic netlist with KiCad as PCB connectivity authority, reconstructs project-local libraries, and emits raw ERC/DRC plus a fail-closed summary. Its replaceable grid adapter rasterizes rotated pads, tracks, vias, exact rectangular or closed-line polygon outlines, and target-pad holes; a deterministic rooted-star baseline handles multi-terminal nets and reserves drill spacing across branches. Persisted candidates replace one net transactionally only after a temporary full KiCad gate. The board-copper importer canonicalizes connected, acyclic, uniform segment/via graphs and fails closed on cycles, dangling material, arcs/zones, and ambiguous cross-layer pad contacts. Rung 7 round-trips four tracks/two vias; fixed-via continuous lowering uses three ordinary same-layer polylines plus two shared fixed anchors, improves 19.243 mm to 18.964 mm, preserves both vias, and native-exact-passes. Per-run uniform-grid selection reproduces the exhaustive candidate byte-for-byte with 51.1x fewer projection rows. Rung 41 imports the real 13-terminal `3V3_DUT` tree as 12 root-to-terminal branches, 44 canonical segments, and 12 vias. A failed zero-force control exposed selected/unselected shared-trunk detachment that KiCad did not flag; eleven fixed context anchors and exact no-op writeback now lower two branches sharing two vias byte-identically. Joint nonzero motion then moves their post-via junction, improves 236.746 mm to 232.763 mm, and exact-passes rungs 41/42. Its one-branch control reaches only 235.943 mm; independent particles split shared trunks and regress to 292.572 mm. Local graph lowering is byte-identical to exhaustive with 11.3x fewer projection rows. A bounded nearest-shared-junction selector expands seed branch 1 to `[1, 2]` after 830 contact tests and reproduces the manual candidate, proposal, and complete board byte-for-byte; a one-branch ceiling rejects before engine work. A source-checked port of `layout-trace`'s remove/relocate-via lifecycle edits all serialized occurrences atomically and native-gates every unique board. Its first 14-action portfolio finds two legal relocation sites; the selected 1 mm move plus tension reaches 236.000 mm versus 236.270 mm for fixed-via tension alone. A bounded analytic-feasible radial frontier then uses 646 cheap geometry probes and diversity suppression to retain four distinct native-valid relocations, reducing the full portfolio to six board gates and reaching 235.950 mm after tension. Its retained preliminary run shows why byte-distinct points on one obstacle boundary are not a useful portfolio. Both direct removal directions remain visible with 19/28 DRC findings, showing that this excursion crosses occupied copper rather than proving removal is generally ineffective. Quality distinguishes the canonical physical copper union from stored overlapping track objects; this retracts earlier retained-vertex and exact-shared-point promotions that optimized the wrong measure. Branch-0 projected tension improves 238.146 mm to 237.552 mm. Selected/context normalization materializes nine contacts and fixes three context nodes; six-branch projected tension reaches 228.465 mm. Its local result is byte-identical to exhaustive with 16.6x fewer projection rows and exact-passes rungs 41/42. Rung-1 placement relaxation lowers motion-reachable courtyard pairs into generic body/body rows. A source-checked port of `testing-esp32-duts`' DRC-driven reference relaxer stores stable field UUIDs and old/new local poses in candidates; its portfolio reconstructs a fresh failure, deduplicates four phases to three candidates, retains all native gates, and selects between two complete sideways actions. Native Rust front/back rendering preserves the predecessor inspection seam. A declared F.Cu/B.Cu GND-zone transaction now replaces all 412 GND tracks, thins its 128-via routed seed to 32, automatically refills zones during verification, and keeps the board exact-complete. Conflict-guided counterfactual cuts show that GND alone opens the branch-8 back-layer repair while no generated front-layer cut of size at most two succeeds. Implementable mixed-action KiCad portfolios, cyclic copper import, via insertion, purpose-built zone stitching, zone-aware repair, broader/adaptive feasible-region relocation, seed selection, repeated graph-neighborhood rebuilding, general multi-component coupling, richer silkscreen envelopes, curved and multi-loop outlines, and a general importer/exporter remain. |
| Viewer/evidence | Best fields from both viewers | Particle/field/body and route/topology-attempt viewers work | The continuous viewer draws analytic rotated bodies, body-local attachment residuals, particles, fields, corrections, and functional playback. The route viewer draws placed bodies, pads, layer-colored copper, vias, explicit junctions, failed legacy intents, exact violations, blocked-frontier centroids, pressure motion, rejected projected poses, multi-body pushes, and retained coordinator attempts. Corridor/family overlays, residual-history plots, richer phase counters, and synchronized side-by-side candidates remain. The viewer remains render-only. |
| Benchmark ladder | Both | Ported and hash-pinned | The reduced routing test has nine exact-gated, work-bounded rungs and the complete C3 ladder remains the native mechanism baseline. The dual-ESP32 JSON generates a 52-connection ladder; every cold prefix prunes connectivity before placement and forbids prior-prefix parents. Prefix 18 is native-complete after the generic order coordinator automatically rediscovers the diagnosed connection swap. Full-width prefix 19 still stops at rung 14 across our order/placement controls, while thin-first priority routing finds a complete 24-branch topology and continuous growth reaches exact 25% width. Conflict-cover selective rerouting reaches 23/24 and proves the leading failure is true `no_path`, not budget exhaustion. A cold exact-rule Freerouting control routes the identical fixed placement at 24/24 in seven passes/5.90 seconds with zero KiCad copper/connectivity findings. This establishes multi-pass rip-up/retry as the missing conventional baseline. Prefixes 19–52 remain without full-width native completion by our engine. |

Sequences 155–158 port the first archived-placement slice from
`testing-esp32-duts`. Placement portfolio schema v2 separates the number of
declared policies from the maximum expanded native trials. Each policy may
retain a bounded, pose-deduplicated cheap archive; rejected attempts remain in
evidence, and every finalist is routed from zero copper before selection. The
review rejected a literal whole-board random population as the useful default:
15/16 proposals remain illegal after four times the legalization budget. A
reviewed local-mutation policy instead starts every proposal from the same
legal base, permits bounded translations and quarter turns, then uses the
common exact projector. It produces 16/16 unique legal dual-board proposals;
the first two routed prefix-5 finalists both native-pass, and the selected one
improves harmonic copper from 153.261470 to 153.103349 mm with the same four
vias. Sequence 159 composes that archive with the already ported blocked-front
pressure search: every finalist is an independent feedback parent, unchanged
parents remain native controls, provisional semantic copper is discarded, and
all four prefix-14 boards native-pass. Feedback saves 0.034160 mm in proposal
2 and 8.594701 mm in proposal 8, although the latter adds two vias; native
selection therefore keeps proposal 2's child. Multi-generation global
selection across newly produced children and richer failed-terminal
neighborhood proposals remain. This slice does not claim that its advisory
placement rank or semantic completion predicts native routing quality.

Sequences 166–193 compose the reviewed routing-order and continuous-engine
seams. The generic cold order search rediscovers the prefix-18 swap and
native-completes 18/18. Prefix-19 full-width order/placement trials plateau at
rung 14. A new replaceable width-continuation coordinator instead routes all
24 branches at 5% declared width, keeping full clearance, and lets only the
continuous engine grow the fixed topology. The first control exact-passes 10%
and exposes unsafe 15% projection. Per-point trust regions, exact rectangular
and circular pad lowering, and finer width stages then reach exact 25%.
Sequence 188 tries trace motion before connected component motion; both stall
at 27.5%, reducing the pressure set to four route pairs for the discrete outer
loop. Conflict-cover and blocker-frontier trials then reach 23/24 before
exposing terminal-via lowering. The external exact-rule Freerouting control
completes 24/24 in seven passes, proving the fixed placement itself is not the
limiting factor. The complete evidence is in
[`experiments/dual-esp32-width-continuation.md`](experiments/dual-esp32-width-continuation.md)
and
[`experiments/dual-esp32-freerouting-baseline.md`](experiments/dual-esp32-freerouting-baseline.md).

Sequences 049–053 extend the KiCad adapter row beyond its rooted-star control.
The adapter now has selectable shared-copper growth, bounded attachment-source
portfolios, explicit length/via/router-cost objectives, optional same-layer
source diversity, and durable searched-source versus actual-join evidence.
The first real four-terminal native insertion removes the rooted-star close-via
pair; its negative diversity and positive nearest-only work ablations remain
retained rather than being collapsed into a new unconditional default.

Sequence 035 supersedes the KiCad-row note that removal with local rerouting is
pending. The implementation deliberately combines, rather than chooses
between, the predecessors: `layout-trace` supplies the atomic physical-via
remove/merge semantics and the `testing-esp32-duts` A* kernel repairs the new
same-layer run. A branch-8 no-path control remains visible at two window sizes;
a branch-9 control removes two vias, reduces canonical copper by 4.015149 mm,
and passes the full native gate. Sequence 036 then ports the DUT router's
blocked-frontier evidence through the KiCad boundary: typed failures retain
lossless row-run occupancy/frontier data and paired board overlays without
making topology-only candidates selectable. Sequence 037 then preserves KiCad
pad/track/via ownership, attributes every real branch-8 frontier cell to a
footprint or foreign net, retains `Router_Movable`, and labels blocker
centroids in the paired overlays. Via insertion, layer-changing repair, and
mixed-action coordination remain pending.

Sequence 038 closes the causal-cut-analysis item without confusing ablation
with implementation. A conflict-guided bounded BFS retries the DUT local A*
after suppressing semantic pad owners or foreign-net copper. New failures can
introduce children absent from the original frontier, as proven by a hidden
two-wall control. The real `F.Cu` run exhausts 50 generated size-one/two cuts
without finding a path; `B.Cu` finds routed `GND` alone sufficient. All trials
are marked counterfactual and non-selectable. Implementable component moves,
foreign-net rip-up/restore, layer-changing repair, via insertion, and
mixed-action coordination remain pending.

Sequence 039 ports the predecessor's missing zone lifecycle rather than
treating the 128-via GND tree as permanent architecture. A declaration-level
transaction removes one selected connection's tracks/zones, optionally thins
its existing vias by board-anchored cells, adds F.Cu/B.Cu zones, and makes
KiCad refilled-zone DRC the acceptance authority. Removing every via leaves
five disconnected islands; retaining 128, 83, 50, or 32 passes, while the
tested 26-via and smaller sets fail connectivity. The selected 8 mm policy
therefore removes all 412 GND tracks and 96 vias, reducing the whole rung to
403 segments/100 vias/two zones while all gates pass. This is the sparsest
passing tested portfolio member, not a global minimum: survivor choice depends
on cell size and serialized via order. Foreign zones now yield during signal
repair while their pads/vias remain hard, and a two-action `3V3_DUT` via
portfolio native-passes both proposals after refill. Purpose-built stitching,
plane-quality scoring, and target-zone graph import remain open.

The KiCad handoff carries source-checked footprint poses. On rung 1, body-local
endpoint attachments and two motion-reachable courtyard pairs move explicitly
movable `R1` by 0.229717 mm under unrestricted X/Y configuration, reduce the
fresh route by 1.443776 mm, survive ladder rematerialization, and pass the
native gate. Preserving distance to all 32 other footprints was investigated
and rejected because it froze the body; the retained motion-envelope broad
phase avoids that architectural failure. Courtyard-constrained free rotation
falls from the unconstrained control's 1.767492° to 0.003058°. Independently
relocating the movable `R2` reference sideways recovers the full rotation and
survives rematerialization, but does not improve route length. The four
predecessor poses also collapse to three unique actions for this reference;
one unique action transfers the collision to `R3`. The native portfolio now
retains and gates all three unique candidates and selects phase 2 after two
passes. General multi-component coupling, mixed action types, and text
readability remain pending rather than being inferred from this small positive
control.

## Retained architecture

- Compiler-style stages and explicit ownership boundaries.
- A discrete coordinator around a continuous legalizer.
- Stable route graphs, shared junctions, layers, and vias.
- Progressive physical-width growth from a topology-valid centerline.
- Transactional candidate commit/rollback.
- Independent exhaustive validation.
- Structure-of-arrays hot state and a CPU reference for a GPU backend.
- Typed pressure, health, work accounting, replay, and viewer evidence.
- Reduced fixtures before full-board tuning.

## Re-evaluated instead of copied

- `BodySoa` and `ParticleSoa` are not assumed to be permanent separate worlds.
  The first compiler particleizes two-terminal and rectangular bodies.
- Pairwise terms called a “field” are not carried forward as a field model. A
  real grid-valued density/congestion seam is explicit.
- Trace chains do not use equality springs that conserve seeded cable length.
  Ordinary trace spacing is a one-sided upper constraint; slack can disappear.
- Field occupancy is not implicitly one unit per particle. Compiler-assigned
  weights keep representation-density experiments from changing the objective.
- Part names and net names never select hot-loop behavior.
- Current CPU loop structure is not treated as the GPU algorithm.
- A lower approximate score is not accepted without exact evidence.
- Viewer logic does not infer correctness.

## Imported benchmark policy

Small inputs are copied verbatim as provenance-preserving historical fixtures.
Their old solver configuration is retained as data but is not automatically
adopted. A system-scale JSON target and a generated KiCad ESP32-C3 project are
retained as distant integration regressions; they do not choose the next
algorithm task.

Tests port invariants and failure mechanisms, not old implementation details.
An old test that asserts a particular correction order or private buffer layout
is replaced by a semantic assertion where possible.

## Unfavorable experiment protocol

Before retiring an approach after a poor result:

1. confirm both variants received the same semantic input, initial candidate,
   exact gates, seed set, and deterministic work budget;
2. distinguish `no_path` from budget exhaustion, integration mistakes, stale
   certificates, and invalid output;
3. compare representation choices and parameter scaling rather than only the
   headline score;
4. inspect seed sensitivity, local minima, and the first divergent iteration in
   the viewer/evidence stream;
5. reduce the failure to the smallest fixture that preserves it and record an
   ablation;
6. involve the project owner when the evidence suggests a genuine conceptual
   tradeoff rather than an implementation defect.

An implementation can be marked inactive after repeated controlled evidence,
but its code, configuration, and counterexample stay available for later
experiments.

The first application of this protocol is recorded in
[`experiments/harmonic-dual-esp32.md`](experiments/harmonic-dual-esp32.md).

## Deferred

- Incremental family-context promotion and terminal escape integration.
- General KiCad import/export beyond the C3 template and generated semantic
  rectangular-board paths.
- Indexed-validator parity and richer fabrication-rule portions of production exact DRC.
- A real `wgpu` compute backend.
- Grow dual-ESP32 beyond the currently complete prefix 18, regenerating
  placement and all routes from zero copper for every prefix. The generic
  bounded order coordinator now rediscovers the prefix-18 swap. Prefix 19 has
  a complete thin semantic topology and exact 25%-width stage, but no
  full-width/native result from our engine. The exact-rule Freerouting control
  completes that fixed placement at 24/24; port the missing repeated
  rip-up/retry behavior, then feed residual pressure back to placement and
  continuous component/trace motion.
- Collinear/via shared-tree contact and synthetic split-branch ownership during
  selective reroute.

These are visible milestones, not silently unsupported input.

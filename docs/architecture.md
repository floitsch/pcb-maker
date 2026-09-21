# Architecture

The product is an autonomous constrained PCB placer and router. Placement,
routing, repair selection, and acceptance must run in the program without an
agent choosing individual board edits at runtime.

Agents are part of the development feedback loop: when the program stalls or
produces poor layouts, inspect the retained board and evidence, diagnose the
limitation, implement an algorithmic improvement, and rerun the case. The layout
analysis experiments guide that diagnosis and implementation. Their inspection
tools are development infrastructure, not a separate product objective.

The compiler and experiment harness below support comparing and improving the
program's strategies. A change should be judged by the resulting
placement/routing capability, quality, and cost on retained cases.

Every new board experiment should retain a rendering with its evidence. Native
verification does this automatically: `preview.svg` overlays front/back copper,
footprint drawings and labels, and the outline, viewed from above.
`preview.json` records the source PCB hash, native result, and render status.
The hook covers full verification, cached verification, zone-refill verification,
and route-only admission, including rejected attempts. Sequential routing copies
the selected preview into its result directory. Preview failures are reported
without changing admission and remove any previous image. These quick native
SVG exports depict stored geometry; they do not overlay DRC findings or refill
zones. Existing high-quality rendering remains available separately.

Retain experiment directories when recording progress. The dual cold-prefix and
via-keepout integration tests support `PCB_MAKER_KEEP_TEST_ARTIFACTS=1`; use it
for development runs whose boards should remain available after the test.
Pure unit tests without a board do not need a fabricated board rendering.

```text
semantic board + fabrication rules
              |
              v
representation compiler <---- experiment configuration
              |
              v
discrete candidate: connectivity, topology, layers, vias, order
              |
              v
continuous world: particles, primitives, constraint batches, fields
              |
              v
independent exact verifier
              |
              +---- typed residuals and pressure ----> coordinator
```

## Ownership boundaries

The semantic model describes electrical and manufacturing meaning. A resistor
is a two-terminal part with dimensions and placement freedoms; it does not say
whether an engine should use an analytic rigid body, two particles, four
particles, or a shape-matching cluster.

A representation compiler chooses the continuous degrees of freedom. The
first available compiler is an experiment, not a selected architecture:

- a two-terminal passive may explicitly opt into two particles plus one
  distance constraint; the default refuses to choose a representation;
- the same passive may instead opt into one analytic body pose plus two
  body-local terminal attachments; translation and rotation mobility are
  independent semantic controls;
- a rectangular rigid part becomes four corner particles plus edge and diagonal
  distance constraints;
- a trace is a particle chain with one-sided maximum-spacing constraints;
- a route endpoint initially reuses its terminal particle.

Particle count is not physical mass. The compiler assigns a separate field
weight and normalizes it per semantic object, so increasing samples does not
silently increase field pressure. This v0 weight is deliberately normalized,
not yet a physical copper/keepout area model. Future pad, segment, via, and
footprint rasterizers belong behind the same field-scatter boundary.

These are experiments, not a claim that either representation is optimal. The
endpoint and analytic-body controls are compared on identical semantic inputs;
a shape-matching particle cluster can be added behind the same boundary.
Solver kernels branch on constraint family, never on component names or part
classes.

The discrete coordinator owns decisions that continuous motion cannot safely
invent without recording: electrical connectivity, pin binding, graph
topology, homotopy or corridor family, layer, via count, lane order, and
transaction rollback. A router or continuous backend may originate a proposal
that changes any of those and may move components as part of that proposal.
The proposal is a candidate transaction: its invalidations are explicit, it is
recertified and exactly validated, and only then is it committed. This lets the
tracer act on placement without making unreviewable mutations.

The verifier is independent of both. Approximate field energy, low particle
motion, or an attractive animation is never evidence of PCB legality.

The native KiCad connection transaction applies this rule to declaration
deltas. It snapshots the exact parent before any external tool runs, verifies
a disposable copy, derives only the next schematic/pad connectivity, and
hydrates that delta onto the parent's existing geometry. Local producers and
future coupled producers all start from that same hydrated artifact. KiCad
ERC, DRC, schematic parity, and selected-net connectivity are the admission
gate. A canonical historical rung may be evaluated as a feasibility control,
but it cannot be selected because it is not guaranteed to preserve the exact
parent. If no parent-derived child passes, the byte-identical snapshot is the
only durable result.

Generated-parent progression never rematerializes a historical board as its
next state. Its local portfolio may contain different algorithms or raster
resolutions; each starts from the same hydrated parent, and selection uses
native completeness followed by physical copper plus via penalty. Native
admission is itself configurable. The exhaustive control gates every generated
candidate. Score-ordered admission retains and ranks every candidate, then
gates until the first complete child; lower-ranked routes remain explicitly
unevaluated and cannot be selected. Resolution refinement precedes topology
escalation because a conservative coarse raster can report a false no-path.
An optional ordered-refinement scheduler can stop a coarse-to-fine portfolio
from live prefix scores after an explicit minimum number of entries. This
changes which candidates are generated, so evidence separately records the
declared portfolio size, evaluated entries, generated routes, and native
gates. A stop never removes recovery capability: if every prefix candidate
fails native admission, all skipped entries are generated and admitted before
any topology-changing fallback.

The KiCad grid adapter keeps multi-terminal topology selectable. `rooted_star`
is the deterministic independent-branch control. `shared_copper_tree` routes
one root branch, then attaches each later terminal to ranked points in already
owned copper. Each trial is trimmed through its last tree contact before
quality comparison, and only the selected trial reserves copper and vias.
Configuration bounds the number of attachment searches, their optional
same-layer physical spacing, and the length/via/router-cost objective. Durable
evidence distinguishes the ranked `searched_source` from the actual trimmed
`source`, and accounts all attempted expansions rather than only the selected
branches. This keeps nearest-only, diverse, and wider portfolios as explicit
experiments instead of embedding one policy in the adapter.

On a fixed-copper failure it may then run a
bounded yielding diagnosis: foreign tracks/vias disappear counterfactually
while pads remain obstacles. A diagnosis is never admissible by itself. The
first topology repair applies the new target route, proves that only one
explicitly yielded old connection is missing, reroutes that old connection
around the target, then repeats the whole native gate. This producer is
optional configuration beside the local portfolio; future two-net,
opposite-order, component-motion, and field-based producers fit the same
attempt boundary.

Continuous engines publish a representation-independent semantic solution
before entering the durable candidate boundary. The current bridge carries
component poses and ordered connection polylines; an adapter rebuilds terminal
nodes and route graphs—including deterministic terminal-MST branches for
multi-terminal nets—and then invokes the same independent exact gates used by
discrete routers. Solver handles never leak into the candidate. Long semantic
edges are normally subdivided to the configured particle spacing; preserving
raw seed vertices is an explicit recovery-test policy.

The dual-board cold-feedback bridge deliberately narrows that handoff further.
It prunes the original semantic input to the requested connection prefix,
regenerates base placement, and lets a bounded router-pressure transaction
select new component poses. Pressure source, component ordering, proposal
direction, relational/collision propagation, beam width, neutral-state
retention, and optional selective rip-up are separate experiment axes. Beam
states are deduplicated by semantic pose rather than route search history, and
each retained parent and child remains in the evidence. Placement proposals
must pass a pad-extended exclusion gate before routing; copper-bearing pairs
receive board-rule clearance, and push propagation uses the same envelopes.
The selected semantic routes are then discarded. A zero-copper KiCad rung 0
is materialized at the selected poses and every requested connection is routed
again through native progression. Its evidence keeps base placement, all
pressure/topology attempts, selected poses and demand, native routing, and
final verification separate; no smaller-prefix layout or provisional route
can silently become a parent. Native completion is also the placement commit
boundary. If a feedback-selected pose stops before the requested rung, the
cold bridge retains that entire trial, independently regenerates the original
base placement and every route from zero copper, and selects the base only if
it reaches at least as far. It never continues from the feedback trial's last
valid partial board. This fail-closed portfolio currently pays for both native
runs; future-connection reachability and exact-result caching may reduce that
work but cannot weaken the final gate.

The first discrete-to-continuous repair handoff resolves selected route-family
clearance obligations into semantic copper-polyline pairs. The engine compiles
these to flat segment-clearance rows, without importing family IDs or route
schemas into its projection loop. Semantic rectangular routing-keepout bodies
in the selected neighborhood compile to a separate flat segment/body row. Its
same effective-mass projection can move either trace particles or the oriented
body's permitted translation/rotation degrees of freedom. Endpoint-owner
bodies are exempt exactly as they are in the independent validator.
Fixed-context copper has zero mobility.
Selected endpoints stay fixed unless they have a body-local attachment; then
projection can transfer force into the component's independent
translation/rotation degrees of freedom. Combined segment/segment and
segment/body expansion has a hard ceiling. The adapter rematerializes
exact semantic endpoints and route-graph nodes after f32 projection and commits
only an exact-complete or Pareto-improved proposal; otherwise it returns the
original candidate and keeps the failed attempt as evidence.

## Experiment interface

`pcb-pipeline` carries the versioned contract ported from `layout-trace`. Every
pass declares prior-pass dependencies, required and established certified
invariants, invalidations, and debts introduced or discharged. The contract is
independent of whether a pass is an in-process Rust implementation, a GPU
dispatch, or a standalone experiment.

The small runtime `Stage<Candidate>` interface still needs transactional
candidate commit and hard budgets before it should coordinate full-board runs.
The same fixture must eventually be runnable with:

- no continuous relaxation;
- particle constraints only;
- particle constraints plus an Eulerian density/congestion field;
- alternative representation compilers;
- CPU and GPU backends;
- different discrete coordinators.

Every comparison records exact findings first, then completion, width achieved,
vias, length, displacement, residuals, deterministic work, time, and memory.
Algorithm and representation IDs, complete configuration, seed, input hash,
work budget, and output hash are part of an experiment record. Wall time alone
is never a fair budget across CPU, GPU, and historical implementations.

## Field hierarchy

The grid is advice and a coarse continuous objective, not geometry authority.
Each copper layer may have several resolutions and channels for occupancy,
routing demand, capacity, congestion pressure, and gradients. Objects scatter
into cells; stencil or multigrid passes solve the field; objects gather forces.

Large boards use overlapping tiles with halos and a coarse board-wide level.
Independent hard tiles are rejected because long nets and component motion
cross their boundaries. The intended decomposition is a global coarse solve,
overlapping local solves, and exact geometric reconciliation.

## Evidence and viewer

Every backend publishes the same frame contract: stable particle/body IDs,
explicit body poses, attachment and constraint residuals, field pressure,
correction vectors, active record/scalar-row work counts, and phase timings.
The viewer only renders this evidence. It does not recompute health or run a
second JavaScript solver.

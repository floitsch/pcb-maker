# Program review after the board-analysis experiments

Product clarification: the deliverable is an autonomous placer/router. Agents
diagnose program failures during development and improve the implementation;
they are not the runtime routing or placement policy. Inspection and local-edit
interfaces below are useful only insofar as they support those program changes.
The next work should start from a reproduced program limitation and finish with
the program handling that situation itself, with a retained regression case.

The recommended working approach is a shared factual board representation,
compact graph and geometry queries, optional focused renderings, explicit local
edits, and native validation of the complete transaction. The experiments support
this as a useful default; they do not establish one universally fastest presentation.

The existing program has valuable foundations: deterministic bounded routing,
physical centerline-union metrics, immutable source snapshots, native checks,
and retained candidate evidence. Preserve those. The immediate work is to make
the factual representation and edit boundary support real existing boards.

Implementation update: the report-execution and structure-validation failures
below have been [repaired](2026-09-07-native-verification-fix.md). This review
records the original findings; coherent board-revision publication and the other
integration changes remain outstanding.

The via-only rule-area omission is also [repaired and covered by an autonomous
routing regression](../../benchmarks/competitive/mechanisms/via-only-keepout.md):
the program now finds the permitted via window and passes native admission
where the unchanged release control stopped after a rejected candidate.

Explicit pad/footprint clearance overrides are now also
[carried through routing and repair](../../benchmarks/competitive/mechanisms/local-pad-clearance.md).
The retained native regression changes from 0/1 to 1/1 committed connections on
the identical board. Project/netclass/custom-rule resolution remains open.

The benchmark and sequential router now also share the base verifier's
[library-metadata classification](2026-09-07-routing-admission-metadata.md).
Changed footprint-library warning records previously appeared as introduced
design violations, producing a false tie in the extended ESP32 comparison.

Duplicate grid geometry queries are now [reused across symmetric transitions](2026-09-07-grid-query-reuse.md).
The retained ESP32 route is identical and natively accepted, with lower CPU
time in repeated route generation. Full-board orchestration still dominates
much of the observed runtime.

Via cleanup now [uses plated terminal layer connections](2026-09-07-plated-terminal-via-cleanup.md)
instead of adding redundant drilled vias at those pads. The retained full
ESP32 board improves from 18 to 17 vias with all 19 connections still natively
valid. General interior pad-mediated graph connectivity remains open.

Initial routing now also [searches the connected layers of plated terminals](2026-09-07-plated-terminal-layer-search.md).
The retained ESP32 connection uses one via instead of two without cleanup;
both reachability and A* handle equivalent endpoints under one search budget.

The router can also [align a coarse grid with a missed narrow pad gap](2026-09-07-pad-gap-grid-alignment.md).
The focused native case crosses a previously missed legal passage without
refining the global grid. It remains an alternate portfolio policy; real-board
benefits and candidate/native length-accounting consistency need further work.

Candidate scoring now also [counts adjacent backtracking copper once](2026-09-07-candidate-copper-union.md).
The gap experiment exposed a candidate/native length disagreement caused by
skipping adjacent segment overlaps. The shared graph fix has native verification,
an independent interval-union check and a retained-candidate audit.

## 1. Fix verification that can incorrectly report success

**Priority: immediate correctness fix. Confidence: reproduced.**

[`run_kicad_report`](../../crates/pcb-kicad/src/lib.rs#L12047) accepts a failed
process whenever the output path exists. Candidate directories are copied with
their earlier reports, so existence does not establish that this invocation
produced the report. [`write_verification_report`](../../crates/pcb-kicad/src/lib.rs#L3835)
also treats absent or incorrectly typed report arrays as zero findings.

Against a freshly built debug executable, isolated fake-KiCad fault injection
demonstrated both cases:

- Both native commands exit 1 without writing reports; pre-existing clean reports
  cause `verify-kicad-rung` to exit 0 and write `complete: true`.
- Both commands write fresh `{}` reports and exit 0; verification again reports
  `complete: true`.

Use fresh temporary report paths, accept only explicitly supported process
outcomes, validate required report structure, and associate verified reports
with exact input hashes. Publish the PCB and its accepted reports as one revision.
The current sequential commit copies the PCB first and three reports afterward,
so an interrupted/failed commit can leave inconsistent artifacts
([source](../../crates/pcb-kicad/src/sequential_router.rs#L435)).

These are failure-handling reproductions; historical native runs were not
reaudited in this review.

## 2. Represent existing copper without requiring it to be an optimized tree

**Priority: prerequisite for the selected workflow. Confidence: reproduced and source inspection.**

The native route importer requires uniform widths/via dimensions, rejects
pad-mediated layer changes, rejects cycles, and rejects dangling edges
([importer](../../crates/pcb-kicad/src/lib.rs#L5250)). The C3 benchmark's GND net
fails import with:

```
target copper graph contains a cycle through [31.375, 42.125] on B.Cu
```

That is precisely a board on which the analysis experiments found useful
topological improvements. A tree-shaped route candidate is a solver output
format; it is too restrictive as the authoritative representation of a board.

Introduce a lossless native board snapshot and a derived physical graph that
allow cycles, branches, partial connectivity, varying widths, and fixed pad
connections. Keep electrical connectivity, geometric contact evidence, and
source-object coverage explicit. Reading a board must not silently simplify it.
Unsupported analysis should produce a coverage limitation while preserving the
native object for round-trip edits.

The lab's source-bound native contact extractor is a concrete starting point:
it gives full modeled routed-net coverage on the C3 example without inventing
new copper. It remains a partial contact model, with explicit limitations.

## 3. Give routing and inspection the same rule-aware geometric facts

**Priority: correctness and search efficiency. Confidence: reproduced and source inspection.**

The Rust routing model is constructed from the PCB expression and uses a single
configured clearance for foreign obstacles
([inflation](../../crates/pcb-kicad/src/lib.rs#L4488)). Obstacles carry ownership
but no resolved object-specific clearance
([model](../../crates/pcb-kicad/src/lib.rs#L10456)). The pad collector does not
read local clearance overrides. The lab's Olimex audit already demonstrated why
this matters: native 1.85 mm mounting-hole clearances cannot be represented by
the uniform default.

Rule areas are only lowered when `tracks` are prohibited
([lowering](../../crates/pcb-kicad/src/lib.rs#L10903)). A minimal adapter fixture
with opposite-layer SMD terminals produces exactly the same candidate with and
without a board-wide `tracks allowed / vias not_allowed` area. Both candidates
place a via at (18.125, 10.125). This checks the Rust adapter, not a fully
materialized native KiCad fixture.

Use distinct permissions for traces, vias, and zone fill, and resolved rule
evidence with each relevant object/pair. For unsupported geometry or rules,
return an explicit limitation or reject the relevant operation. Do not interpret
missing knowledge as free space. Native validation remains the final authority;
the query model should prevent predictable rejected candidates.

## 4. Make local edits explicit, revision-bound transactions

**Priority: integration prerequisite. Confidence: source inspection and design recommendation.**

[`apply_route_candidate_with_yielding_connections`](../../crates/pcb-kicad/src/lib.rs#L5618)
removes all target/yielded net segments, arcs, vias, and zones before adding
replacement tracks/vias. The route candidate has no board revision hash; source
pose checks apply to explicitly moved footprints/reference fields. This is useful
for provisional whole-net routing, but insufficient as the general local edit API.

Add a patch format with the base revision, exact removed/modified object IDs,
added geometry, and explicit allowed scope. Preserve unmentioned native objects,
including zones and rules. Compose dependent changes across nets into one patch.
The materializer must track partial source coverage so deleting a source track
does not delete a required portion elsewhere. Reject stale patches before work.

Use the existing immutable-parent and native-admission machinery behind this
boundary. Cold sequential routing can remain a benchmark, and ladder insertion
can remain a client; general improvement should operate on the current board.

## 5. Consolidate the factual services and keep policy separate

**Priority: performance and maintainability. Confidence: source inspection and experimental evidence.**

`pcb-kicad/src/lib.rs` has 15,418 lines and the CLI has 5,711. More significantly,
the KiCad adapter, semantic router, and Python analysis lab have separate geometry,
contact, and materialization implementations. The generic `StageContract` type
has no Rust callers outside its own module/tests. More framework declarations
will not fix the actual integration boundary.

Extract cohesive native import/rules, graph, query/rendering, patch, and verifier
services as the integration proceeds. Keep solver-specific models as derived
adapters rather than forcing all algorithms into one representation. Prefer
native integer coordinates for authoritative identity; floating-point solver
views can remain derived.

Build spatial indexes and graph facts once per immutable revision. Support batch
queries and keep request/result IDs stable across numerical and visual views.
The lab's 64-query calibration reduced process overhead from 3.78 s to 0.329 s;
the agent spent some saved time exploring more candidates, so this is not an
established end-to-end agent speedup. Rust routing currently reparses and
rasterizes the board for each call. A shared session/index and explicit work
budgets are more directly motivated than GPU work at this point.

Let deterministic code perform geometry, connectivity bookkeeping, and patch
materialization. Let the agent choose where to investigate, generate hypotheses,
and coordinate strategies. Repeated successful optimization operations can then
be promoted into deterministic program features.

## 6. Define improvement beyond a length/via scalar

**Priority: before broad automatic acceptance. Confidence: metric definition and experimental evidence.**

The program already distinguishes physical centerline union from serialized
track length. Preserve that correction. Length plus a via penalty is still a
limited objective: it does not describe ground return paths, timing, current
capacity, or the reason for deliberately redundant copper.

Use explicit hard constraints/protected routing and report separate quality
deltas. Apply scalar weights only within an eligible scope. Initial integration
should exercise ordinary signal-route cleanup with protected zones and sensitive
net classes. Ground-network deletion requires an explicit policy beyond native
DRC acceptance. The lab's largest ground reductions are geometric benchmark
results, not evidence of improved electrical performance.

## Recommended implementation order and validation

1. Repair report execution/validation and coherent revision publication.
2. Establish a source-bound snapshot and graph that ingest the existing C3
   board, including its cycles, without changing geometry.
3. Add the shared query interface and revision-bound patch materializer; fix
   clearance/keepout coverage for the supported scope.
4. Integrate one signal-route cleanup operation through that complete path.
5. Expand to via relocation, coordinated reroutes, and placement changes using
   the same transaction boundary.

The first acceptance checks should cover failed/malformed native reports,
unchanged-geometry normalization, local-rule blockers, stale patch rejection,
partial source-track retention, protected objects, and genuine native-validated
improvement. Existing experiment fixtures supply useful regressions; no new
agent trials are needed to start implementation.

Review validation: `cargo build --bin pcb-maker` passed; 79 `pcb-kicad` and
9 `pcb-grid-router` library tests passed. All 55 analysis-lab unit tests passed.
The targeted existing suites do not cover the reproduced failures. The
fault-injection script and outputs are retained in
[`build/program-review/2026-09-07`](../../build/program-review/2026-09-07/reproduce.py),
with machine-readable [results](../../build/program-review/2026-09-07/results.json).
Main Rust code was not modified during this review.

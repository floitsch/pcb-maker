# Continuous family-obligation repair

## Question

Can a route-family assignment retain a topologically acceptable but slightly
under-clearance pair, hand only that explicit work to the continuous engine,
and accept the result without weakening exact validation?

## Implemented boundary

The routing adapter resolves selected family IDs and fixed-context IDs back to
their full-width same-layer polylines. It then emits a representation-neutral
request containing copper polylines, mobility, widths, pair separations, board
bounds, and an explicit segment-pair ceiling. `pcb-engine` compiles that into
flat particle buffers plus one `segment_clearance` SoA row for every segment
pair. Nearby semantic body keepouts compile to a separate
`segment_body_clearance` SoA family. Its row contains two particle handles, an
analytic oriented-body handle, and a minimum distance; both trace particles
and the body's translation/rotation degrees of freedom participate through
generic effective mass. The GPU dispatch contract therefore has distinct
`ProjectSegmentClearance` and `ProjectSegmentBodyClearance` families; the
implementation exercised here is still the CPU reference backend.

Selected trace endpoints are body-local attachments. Clearance projection can
therefore transfer force through a terminal into a component's independently
declared x/y/rotation mobility instead of detaching copper. Fixed components
and fixed-context runs have zero mobility. After solving, exact terminal
coordinates and route-graph nodes are rematerialized from the component poses.
Continuous motion clears the old route-basis fingerprint because its placement
epoch is no longer authoritative; the durable result claims only what the
independent exact geometry/electrical gate re-establishes.
The expansion is the Cartesian product of segment counts for every obligation,
plus one row per selected segment/body pair, and fails before the combined
default 100,000-pair ceiling. Body contacts are an explicit ablation switch.

The f32 solver targets 1 µm beyond the exact clearance by default. This
explicit numerical margin prevents a converged f32 position a few nanometres
inside the f64 exact threshold from being mistaken for a useful result. It is
configurable and the exact validator remains authoritative.

Acceptance is transactional. The proposal is exact-validated independently
and is retained only if it is complete, or if both trace-clearance findings and
total findings are non-increasing with at least one strict improvement.
Otherwise the returned candidate is the unchanged input; the rejected exact
assessment and frames remain available for diagnosis.

## Natural reduced evaluation

`benchmarks/small/family-clearance-obligation.json` has two straight 0.5 mm
traces whose centers are 0.70 mm apart against a 0.75 mm requirement. Each
finite visibility product graph exhausts after exactly one certified class.
The imported producer previously rejected those complete domains because they
did not contain an arbitrary minimum of three families. It now accepts an
exhausted short frontier only when every returned raw class was successfully
certified; truncated or post-certification-short frontiers still fail closed.

Joint assignment naturally emits one `CopperClearance` obligation. The lower
trace's two endpoint bodies may move vertically, so the one segment constraint
pushes both bodies from y=3.000000 mm to y=2.949060 mm. The independent exact
gate then reports zero geometry and electrical findings.

| Measure | Result |
| --- | ---: |
| Certified domains | 1 family per branch, both search-exhausted |
| Visibility states | 60 |
| Continuous obligations | 1 |
| Segment-pair rows | 1 |
| Frames | 9 |
| Initial solver residual | 0.051000 mm |
| Final solver residual to margin target | 0.000060 mm |
| Component displacement | 0.050940 mm each |
| Exact findings | 0 |

A matched negative control fixes all four endpoint bodies. The same naturally
produced obligation remains unresolved and the transaction rolls back with its
one exact clearance finding. This distinguishes missing physical freedom from
insufficient iteration count.

Run and inspect the persisted control with:

```sh
cargo run -- analyze-route-families \
  benchmarks/small/family-clearance-obligation.json \
  declared run/family-clearance-obligation.json
cargo run -- view-continuous-repair \
  run/family-clearance-obligation.json run/family-clearance-obligation.html
cargo run -- validate \
  benchmarks/small/family-clearance-obligation.json \
  run/family-clearance-obligation.json
```

## Local body-neighborhood evaluation

`benchmarks/small/family-clearance-body-neighborhood.json` adds a fixed
4.0 × 0.1 mm routing-keepout body exactly 0.50 mm below the lower trace and
makes both trace endpoint pairs vertically movable. The family stage again
naturally emits one trace-pair obligation. Blindly splitting its 0.051 mm
correction between the traces would push the lower trace into the keepout.

With body contacts enabled, one trace-pair row and two trace/body rows keep the
lower endpoints at y=3.001010/3.000996 mm and move the upper endpoints to
y=3.751982/3.751961 mm. Exact validation reports zero findings. All nine frames
retain the body and `segment_body_clearance` residuals, and the viewer now
draws constraints whose second handle is a body.

The matched `include_body_contacts=false` run uses the same input, families,
assignment, steps, and numerical margin. It removes the trace/trace finding but
creates one `trace_obstacle_clearance` finding, so the route-family result stays
`ExactRejected`. This is direct evidence that neighborhood contact prevents
conflict substitution.

| Measure | Contact enabled | Contact ablated |
| --- | ---: | ---: |
| Visibility states | 413 | 413 |
| Family obligations | 1 | 1 |
| Trace-pair rows | 1 | 1 |
| Trace/body rows | 2 | 0 |
| Retained frames | 9 | 9 |
| Exact findings | 0 | 1 trace/body |
| Result | exact complete | exact rejected |

```sh
cargo run -- analyze-route-families \
  benchmarks/small/family-clearance-body-neighborhood.json \
  declared run/family-clearance-body-neighborhood.json
cargo run -- view-continuous-repair \
  run/family-clearance-body-neighborhood.json \
  run/family-clearance-body-neighborhood.html

# Expected to exit nonzero after persisting the rejected ablation evidence.
cargo run -- analyze-route-families \
  benchmarks/small/family-clearance-body-neighborhood.json \
  declared run/family-clearance-body-contact-ablated.json \
  benchmarks/small/family-clearance-body-contact-ablated.json
```

## Injected local-shape evaluation

The selected-family positive control injects a local clearance shortfall into
the already-small power/capacitor slice and attaches an obligation using the
actual selected family IDs. Four segment-pair constraints reduce the exact
score from 4 trace-clearance / 6 total findings to 1 / 3. The final constraint
residual is zero. This is evidence that the handoff, projection, semantic
materialization, and exact acceptance gate work; it is not evidence that the
current projection solves every shape around every obstacle.

The older negative control replaces the movable route with a two-point trace
while keeping its endpoint bodies fixed. The constraint remains unresolved and
the transaction returns the original candidate unchanged.

Every attempt records an initial pre-projection frame followed by eight
correction frames, so pressing Play visibly transitions from the defect to the
attempt instead of showing several already-converged frames. Given a serialized
family result with an attempted correction, render it with:

```sh
cargo run -- view-continuous-repair \
  run/route-family-result.json run/continuous-family-repair.html
```

## Evaluation

Advantages:

- preserves the discrete family/homotopy decision while moving geometry;
- maps directly to parallel flat buffers rather than component-specific code;
- handles movable/movable and movable/fixed copper with one constraint type;
- handles trace/fixed-body and trace/movable-body response with one oriented
  body constraint type, including angular response;
- retains bounded work, initial/final visual evidence, exact proposal evidence,
  and rollback.

Penalties and current gaps:

- work is quadratic in the two polylines' segment counts until a broad phase is
  added;
- this continuous obligation path only handles same-layer via-free runs; V5
  portfolios can materialize vias, but those transitions are not yet lowered
  into continuous correction constraints;
- semantic rectangular body keepouts are covered, but explicit pad/keepout
  shapes, body/body contacts, vias, and third traces outside the named family
  obligations are not yet local contact rows;
- current trace/body expansion is exhaustive over selected traces and body
  keepouts rather than generated by a dynamic broad phase;
- no GPU timing, occupancy, or crossover measurement exists yet.

The experiment now demonstrates the intended tracer-to-placer feedback on a
naturally generated obligation, but it is not complete. The next discriminating
controls are a rotation-driven exact end-to-end correction, adaptive
subdivision when bodies are fixed but interior shape freedom exists, explicit
pad/keepout contacts, and a denser neighborhood with broad-phase generation.

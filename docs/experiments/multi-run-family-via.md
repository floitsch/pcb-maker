# Native multi-run route families

## Question

Can the Rust family producer discover, certify, select, materialize, and show a
route whose ordered runs change copper layers? Can it retain useful evidence
when an obvious via site is illegal?

This is a reduced mechanism experiment. It does not claim general multilayer
board routing.

## Fixtures

`benchmarks/small/family-multi-run-via.json` has two fixed terminals. `A.p`
exists only on `top`, `B.p` exists only on `bottom`, and `LINK` permits both
layers. The native producer samples five bounded positions along the seed. For
each legal position it independently searches and re-embeds a top run and a
bottom run. A checked-in V5 portfolio separately tests the external file
boundary.

`benchmarks/small/family-seeded-via-obstacle.json` adds a 3 × 3 mm keepout
centered on the straight seed's midpoint. That midpoint is an attractive but
illegal via location. Four other sampled locations remain legal.

`benchmarks/small/family-seeded-via-two-net.json` places two parallel nets
0.85 mm apart. Their 0.3 mm traces and every via-to-trace pair are clear, but
two 0.8 mm vias at the same sampled x position need 1.0 mm centerline spacing.
This isolates cross-variable via assignment from ordinary trace conflicts.

## Result

The clear fixture exact-passes with five spatial families, 10 run searches,
and 20 visibility states. The selected candidate has ordered segment layers
`[top, bottom]`, one 0.8/0.4 mm directed via, and a matching route-graph via
decorator.

The obstacle fixture exact-passes with eight admitted families spanning four
distinct via positions. It performs eight run searches and 114 visibility
states. The midpoint is absent from all families, and the retained rejection
states that `(10,6)` needs a 0.6 mm radius while both layer-clearance minima are
zero. Spatial round-robin admission is important here: filling the eight-family
limit from the first site's homotopy combinations hid the other positions.

The two-net fixture exact-passes after 20 run searches and 390 visibility
states. It admits 13 families, exhaustively classifies 40 cross-variable
family pairs, retains 20 conflicts, and selects vias at different x positions
with no repair obligation. Restricting both variables to their single midpoint
site performs four run searches and proves the admitted portfolio bounded
infeasible. This is not a claim that no route exists outside that one-site
frontier.

Durable viewer evidence is stored in:

- `artifacts/small/family-multi-run-via.native.result.json`;
- `artifacts/small/family-multi-run-via.native.html`;
- `artifacts/small/family-seeded-via-obstacle.result.json`;
- `artifacts/small/family-seeded-via-obstacle.html`;
- `artifacts/small/family-seeded-via-two-net.result.json`;
- `artifacts/small/family-seeded-via-two-net.html`;
- `artifacts/small/family-seeded-via-two-net.one-site.result.json`.

The earlier imported-boundary result remains in
`artifacts/small/family-multi-run-via.result.json` and its adjacent HTML file.

## Certificate and fail-closed controls

V5 serializes physical via diameter/drill, the exact clearance model, and the
certified minimum clearance. Validation rejects missing geometry, drill larger
than copper, a clearance shortfall, a transition kind which disagrees with its
layers, a disconnected transition, and run ranges which do not exactly cover
their witnesses. Materialization also requires via geometry to match the board
rules.

Family conflict rebuilding classifies every run and each via annulus against
every cross-net family and fixed-context copper primitive. A control places
foreign copper close enough to conflict with the annulus but not the thinner
run and proves that both family and context conflicts are found.

The producer obeys the configured settled-state and visibility-work limits. A
zero-budget control stops after its first run search and returns typed
portfolio-unavailable evidence. Disabling native seeded-via production is a
matched negative control and cannot solve the different-layer endpoint case.

Clearance shortfalls have an explicit assignment policy. The predecessor
behavior keeps topology-compatible shortfalls as continuous repair obligations.
The native multi-run producer defaults to discrete separation because its
continuous handoff does not yet support vias. A matched policy control proves
that selecting repair-obligation mode returns a durable exact-rejected result
instead of aborting analysis. This keeps both algorithm choices available
without pretending the unsupported repair succeeded.

## Evaluation

The experiment works at its intended boundary. A native family no longer has
to flatten a layer transition into misleading single-layer copper, and using
ordinary ordered runs plus transition records avoids a special via-bearing
trace type. The representation is suitable for batched processing because the
extra structure is proportional to runs and transitions.

The current search is intentionally narrow:

- exactly one transition and two runs when endpoint pad layers differ;
- via positions only at internal seed vertices and bounded arc-length samples;
- no general two-dimensional via lattice or corridor-derived adaptive sites;
- no multiple-via or independently chosen layer sequence;
- only one deliberately parallel interacting-net control, without obstacles
  determining different lateral via choices.

Those limits make the next experiments clear. First add corridor-derived
two-dimensional sites and an interacting two-net obstacle control. Then test
multiple transitions. A poor result should be reduced and inspected through
the retained site/run/family evidence before judging the approach.

## Reproduce

```sh
cargo run -- analyze-route-families \
  benchmarks/small/family-multi-run-via.json \
  declared run/family-multi-run-via.native.result.json
cargo run -- view-route \
  benchmarks/small/family-multi-run-via.json \
  run/family-multi-run-via.native.result.json \
  run/family-multi-run-via.native.html

cargo run -- analyze-route-families \
  benchmarks/small/family-seeded-via-obstacle.json \
  declared run/family-seeded-via-obstacle.result.json
cargo run -- validate \
  benchmarks/small/family-seeded-via-obstacle.json \
  run/family-seeded-via-obstacle.result.json
cargo run -- view-route \
  benchmarks/small/family-seeded-via-obstacle.json \
  run/family-seeded-via-obstacle.result.json \
  run/family-seeded-via-obstacle.html

cargo run -- analyze-route-families \
  benchmarks/small/family-seeded-via-two-net.json \
  declared run/family-seeded-via-two-net.result.json
cargo run -- view-route \
  benchmarks/small/family-seeded-via-two-net.json \
  run/family-seeded-via-two-net.result.json \
  run/family-seeded-via-two-net.html

# Matched negative: both variables receive only the conflicting midpoint.
cargo run -- analyze-route-families \
  benchmarks/small/family-seeded-via-two-net.json \
  declared run/family-seeded-via-two-net.one-site.result.json \
  experiments/configs/route-family-seeded-via-one-site.json

# Independent external-portfolio boundary.
cargo run -- materialize-route-family-portfolio \
  benchmarks/small/family-multi-run-via.json \
  benchmarks/small/family-multi-run-via.portfolio.json \
  declared run/family-multi-run-via.imported.result.json
```

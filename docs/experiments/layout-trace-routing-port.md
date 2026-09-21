# Layout-trace routing-kernel port

## Scope

The extracted `layout-trace-routing` crate is now a first-class workspace
library. Its original module boundaries are retained:

- constrained-Delaunay corridor construction, semantic reduction, cut bases,
  exact-clearance embedding, and bounded visibility repair;
- exact fixed-copper pair classification;
- versioned route-family portfolio IR and validation;
- bounded route-family generation and exact joint assignment;
- fixed-context correction; and
- placement-independent combinatorial routing regimes.

This remains independent from the DUT raster router. Both consume the same
`layout-trace-model`, and only adapters convert their results to the shared
candidate/exact-validation boundary.

The initial imported Rust source matched the predecessor byte-for-byte after
the local license header, except for the fixture include path in `family_ir.rs`.
The first deliberate kernel extension is small and isolated: the portfolio
entry point can now forward the kernel's existing caller-controlled visibility
work budget. The work-budget type also derives `Deserialize` for experiment
configuration. Its required fixed-context evidence fixture is hash-pinned in
the benchmark manifest. All 149 original routing-kernel tests still pass here.

## Adapter behavior

`analyze-corridors` converts an exact-valid placement into per-layer polygonal
obstacles, builds current corridor epochs, and embeds every deterministic
branch independently. Full-width success requires exact obstacle clearance and
the same current-epoch homotopy word. A geometrically clear path in a different
homotopy class is retained but not called successful.

`analyze-route-families` then generates bounded alternative families, checks
every cross-net witness pair, runs exact joint assignment, materializes the
selected witnesses as the common `CandidateArtifact`, and runs the independent
geometry and electrical gates. A family selection is not complete unless that
materialized candidate passes both gates.

`repair-route-families` applies the predecessor's bounded-component policy to
an existing candidate. It selects the smallest connected component of exact
same-layer trace conflicts, limited to six eligible branches by default, and
passes every outside copper run and via annulus as immutable fixed context.
The proposed replacement is retained for inspection, but is only accepted when
its independent exact score improves. The component limit is configuration,
not a hidden architectural assumption.

## Results

| Case | Corridor result | Family/candidate result |
| --- | ---: | ---: |
| forced two-layer crossing | 2/2 independent full-width embeddings; 76 cells, 78 gates | not needed for this smoke case |
| explicit three-terminal tree | 2/2 independent full-width embeddings; 58 cells, 59 gates | not needed for this smoke case |
| west power/capacitor slice | 3/4 preserve the seed homotopy; the fourth has a clear alternate class | 4 variables x 3 certified families; 36 family pairs; selected in 8 assignment nodes; exact candidate passes 0 physical/0 electrical findings |
| dual ESP32 harmonic pose | 34/83 preserve the seed homotopy; 1,834 cells, 2,016 gates | bounded unavailable on the first hard branch (`CTRL_E_BOOT`) when visibility construction reaches its graph limit |
| dual ESP32 negotiated candidates, default 25M geometry-unit ceiling | fixed-context repair selects a 2-branch conflict component instead of all 83 intents | three retained candidates stop on their first selected branch at `VisibilityGraphLimitReached`; assignment is never entered |
| same three candidates, opt-in 100M geometry-unit ceiling | same exact fixed context and 2-branch selection | one improves selected conflicts 3→1 and findings 29→28; one proves bounded infeasible after deeper portfolio stages; one still reaches the visibility limit |

The power slice is the first exact-complete candidate produced through the
ported corridor/family architecture. It is also a useful counterexample to
accepting the initial independent embeddings: `W_CB_G` is geometrically
repairable only by changing its cut word. The joint family pass makes that
discrete choice while avoiding the other three routes.

The dual-board failures separate cleanly:

- 34 branches retain their seed homotopy with full-width clearance;
- 36 have certified clear visibility alternatives in another homotopy class;
- 8 cannot escape the selected source corridor cell at required clearance; and
- 5 exhaust or hit the bounded visibility graph while preserving the requested
  class.

Running all 83 variables as one initial family portfolio stops at
`CTRL_E_BOOT` before assignment. This does not condemn the family approach:
bounded conflict-component selection and accepted-copper fixed context are now
ported. With the predecessor's default 25M geometry-unit ceiling, three
negotiated dual-board candidates correctly reduce the request to two branches
but stop before assignment. An explicit 100M-unit ablation changes the result:

- the `WEST_AUX_C_HEADER`/`WEST_AUX_D_HEADER` component generates six families,
  enters assignment, and improves selected conflicts 3→1 (29→28 total exact
  findings) using 83,902,376 visibility geometry units;
- a second pass on that result proposes 1→1 conflicts and worsens total
  findings 28→30, so it is rejected rather than committed;
- the `SIG01_TAP_R`/`SIG05_TAP_R` component reaches bounded infeasibility after
  30 certified families and three assignment searches; exact context
  correction proves that promoting up to three eligible outside branches
  cannot make this admitted portfolio feasible; and
- the `EAST_AUX_A_HEADER`/`EAST_AUX_B_HEADER` component still reaches the
  visibility limit after five family searches and 376,462,768 accumulated
  geometry units.

So the default ceiling was a real limiter, but merely spending more is not a
general solution. The next priorities are correcting explicit family-pair
repair obligations, promoting implicated fixed context, and reducing the
large-board visibility graph. Raising the assignment-node budget would not
address any of these observed failures.

## Reproduce

```sh
cargo run --release -- analyze-corridors \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  declared run/forced-corridors.json \
  experiments/configs/corridor-full-width.json

cargo run --release -- analyze-route-families \
  benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json \
  declared run/power-families.json \
  experiments/configs/route-family-default.json

cargo run -- validate \
  benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json \
  run/power-families.json

cargo run -- view-route \
  benchmarks/imported/layout-trace/esp32-slices/power-west-capacitor-interaction.json \
  run/power-families.json run/power-families.html

cargo run --release -- repair-route-families \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json \
  run/dual-negotiated.json run/dual-family-repair.json \
  experiments/configs/route-family-default.json

# Repeat with an explicitly larger exact-visibility work budget.
cargo run --release -- repair-route-families \
  benchmarks/imported/layout-trace/dual-esp32-benchmark.json \
  run/dual-negotiated.json run/dual-family-repair-deep.json \
  experiments/configs/route-family-deep-visibility.json
```

## Remaining integration

- Promote implicated fixed-context branches incrementally when the selected
  bounded component has no feasible family.
- Generalize multi-run/via production. V5 now exact-materializes ordered runs
  and directed vias from both imported portfolios and the native bounded
  seed-site producer. The latter currently supports one transition between
  endpoint-required layers; adaptive two-dimensional sites, multiple
  transitions, and interacting-net controls remain.
- Port terminal escape, flexible contact selection, and corridor-driven
  placement/topology events.
- Show corridor regions, gates, cut words, family alternatives, and assignment
  conflicts in the viewer rather than only the selected copper.

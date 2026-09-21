# Automatic demand forecasting and learned net order

`route-kicad-board-adaptive` integrates the successful demand/order experiments
into the native program. It generates its own forecasts, evaluates bounded
distinct policies, diagnoses observed routing difficulty and proposes new
orders. The caller supplies a cold project and routing rules, not forecast
paths, target coordinates, difficult-net labels or a selected solution.

```sh
pcb-maker route-kicad-board-adaptive SOURCE_DIRECTORY BOARD_ID OUTPUT_DIRECTORY CONFIG.json
```

The source must explicitly contain zero routed copper and have no native ERC,
design DRC or schematic parity issues. Expected unconnected items and separately
classified library metadata warnings are allowed. Output must be fresh and
outside the source, with an existing parent directory. This command fits after
placement/cold preparation; it does not silently erase an input routed board.

## Configuration

```json
{
  "sequential": {
    "connection_order": [],
    "routing_portfolio": [{}],
    "maximum_connections": 1024,
    "via_penalty_mm": 2.0
  },
  "maximum_passes": 6,
  "optimize_after_routing_complete": false,
  "demand_strengths": [1.0],
  "demand_shoulder_mm": 1.0,
  "forecast_via_costs_mm": [null, 20.0],
  "maximum_forecasts_per_net": 2
}
```

The example shows defaults, not manufacturing recommendations. Use the actual
board's routing rules and per-net classes in `sequential.routing_portfolio`.
An empty connection order uses native discovery order by default. Optional
`initial_order: "electrical_terminal_count_descending"` routes nets with more
electrically distinct terminals first, preserving discovery order for ties.
It uses existing source discovery without route forecasts; coincident terminals
on electrically separate copper layers count separately. The order resolves
once for the initial pass, and subsequent learned orders retain their normal
behavior. A nondefault automatic order conflicts with an explicit
`sequential.connection_order` and is rejected before routing. This is an
experimental heuristic, not a guarantee that high-fanout nets are hardest.
`maximum_connections`
must permit the entire net inventory. Explicit `routing_demand` inputs are
rejected because this coordinator owns forecast generation.

Optional `initial_demand_strength: 1.0` generates cold-board forecasts before
the first committing pass and applies their existing soft routing costs to that
pass. It preserves the configured net order and attachment objective. The value
must be finite, positive and at most 1,000,000; omission retains deferred
forecasting and the original first pass. Forecast generation also runs for a
one-pass experiment and must be charged to its total runtime budget. Missing
forecasts remain visible in the evidence; an empty forecast set falls back to
the original policy. Subsequent trials retain their normal bounds, deduplication
and unguided alternatives. This is an experimental congestion policy, not an
established whole-board completion improvement.

A pass normally stops at its first failed net. Opt in to
`sequential.continue_after_routing_failure: true` to attempt later nets once,
with the same cumulative per-pass repair allowance. See the
[bounded sweep contract](2026-09-09-in-process-sweep.md) for sparse electrical
inventory, failure classification and checkpoint compatibility.

Current default behavior stops after the first native-admissible result with
zero opens. `maximum_passes` is an upper bound. Set
`optimize_after_routing_complete` to `true` to reproduce the multi-pass
quality searches reported below. With provisional annotation opt-in, routing
can stop while full-layout admission remains false; remaining findings are
still reported and the CLI still returns failure for that unfinished layout.

`null` in forecast via preferences uses the legacy integer via cost; a number
sets a physical track-equivalent via cost. Independent alternatives are
deduplicated by their layered branch paths and share the connection's forecast
weight. Failure/coverage evidence remains explicit. The best independent route
quality under the configured whole-board via penalty supplies a reference for
cost inflation; it is a heuristic reference, not an optimality lower bound.

## Search and retention

The initial queue contains the original configured policy, a router-cost
attachment control where that differs, and each configured demand strength.
Every pass starts from the identical zero-copper project. The coordinator
compares a net's actual route cost with its independent forecast and promotes:

1. Failed nets.
2. Nets the stopped pass never reached.
3. Routed nets lacking a successful forecast, explicitly marked unknown.
4. Routed nets by decreasing added length-plus-via cost.

Numerical differences below 1 nm of track-equivalent cost are rounded away.
New orders are tried with and without demand. Exact repeated configurations
are not queued again. Demand variants are skipped when there are no forecasts
or no foreign net, since they would reproduce the control's zero cost field.

Every retained connection still passes the existing sequential native gate.
Each whole pass also receives full native verification. Before selection,
all non-copper board records, project bytes and schematic/ERC input hashes
must remain unchanged. Selection prioritizes native design cleanliness and
fewer unconnected items over physical copper length plus the explicitly
configured via penalty. A complete incumbent therefore survives later failed
or incomplete routes, even when those routes contain less copper.

If no pass improves the empty source, `result/` retains its exact board bytes
and the command returns nonzero with `complete: false`. Otherwise it copies
the retained project and independently re-verifies the final copy. Reaching
`maximum_passes` means a bounded run ended, not that routing is globally optimal.
Queued-trial counts and termination reasons are recorded.

## Verification evidence

The ECC83 six-pass integration starts from the original ten-via policy and
automatically retains pass 2: **3 vias, 345.004472 mm**, fully connected with
zero native design/ERC/parity issues. The final tested pass has two unconnected
items and a lower raw route score; it correctly does not replace the complete
incumbent. Seven library metadata warnings remain separate.

The failure-recovery control starts with the previously unsuccessful order.
Its first pass leaves one open; the command recovers to a complete **5-via,
347.590259 mm** board without supplied repair targets. Both learned-order
alternatives subsequently complete but cost more, and are rejected.

A deliberately one-expansion search budget makes every independent forecast
fail. The final implementation tests two distinct policies, reports incomplete
routing and preserves the cold board exactly, including its 20 expected opens.
It does not describe bounded search failure as geometric impossibility.

The final executable also completes the PIC holdout: three automatically
generated policies, with pass 2 selected at **10 vias, 1826.835330 mm**. All 34
nets are connected, all native findings are zero, and original per-net trace
widths, via dimensions and project rules are preserved. The initial configured
policy reproduced 18 vias / 1826.911368 mm.

All 116 KiCad library tests pass on the initial implementation. The final
three adaptive tests pass after no-op-policy elimination and forecast artifact
cleanup. Tests cover completion-first retention, exclusion of native-invalid
boards, repeated-policy suppression and failed/unattempted/costly-net ranking.
An independent audit checks all five recorded runs, reproduces incumbent
selection, reads back native placement/net/copper dimensions, checks 604 SVGs
and verifies all 190 player frame paths plus JavaScript syntax. Headless Firefox
failed at startup in this environment, so browser playback is not claimed.

## Artifacts and inspection

- `adaptive-routing.json`: final result, forecasts, pass configs, observed
  difficulty, selection, source hash and limitations of the stopping condition.
- `forecast-evidence.json` / `forecast-alternatives.json`: independent search
  outcomes and the generated spatial forecasts.
- `passes.json` / `progress.json`: updated after each completed pass, including
  the retained result and remaining queue size.
- `forecasts/`: each provisional forecast and an automatic labelled copper
  preview. These are not native-certified layouts; no copied source DRC/ERC/
  verification report is left beside changed forecast geometry.
- `pass-*/`: native sequential routing attempts and verification renders.
- `result/`: the retained complete board, or honestly reported partial/source
  fallback, plus final native verification and rendering.
- `index.html`: completed-pass table and a routing-step player. The frames show
  committed routing decisions, not spring/repulsion dynamics.

Comparison pages now lead with **combined front/back copper**. The inspection
renderer emits `inspection-combined.svg/png` with translucent red/blue layer
groups in front-view coordinates, keeping component bodies hidden and labels
opaque. Separate front and mirrored-back views remain available for detail.

[Integration comparisons and audits](../../build/adaptive-routing-2026-09-08/index.html)
include executable provenance. The initial ECC83 runs used the first frozen
binary; the final binary adds no-op demand suppression and removes inherited
cold-board reports from provisional forecast directories. Historical copied
reports were retained under explicit `inherited-cold-source-*` names, with a
retirement manifest, so they cannot be mistaken for forecast certification.

## Remaining work

Forecasts are still static empty-board embeddings; only committed-net removal
and ordering respond within this loop. There is no learned local congestion
history, measured passage-capacity reservation, or prefix reuse yet. All
reordered passes restart from the cold source. The queue is bounded and
deduplicated within one invocation, but it is not resumable across invocations.
The successful order policy is based on full-pass evidence; the earlier
pairwise-only ranking is intentionally not a default here.

These are routing improvements at fixed placement. The broader autonomous
placer/router objective still requires placement-area search and additional
board families, alongside more powerful congestion and topology analysis.

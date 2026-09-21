# Observed-failure routing order

This native mechanism control tests whether the program can revise an early net
order after observing a failed connection. It has four fixed top-layer pads on a
10 × 8 mm board, two crossing connections, and a bottom-layer track/via keepout.
The horizontal pads and their shortest trace form a clearance barrier between
the board edges. Routing the interior vertical connection first leaves room for
the horizontal connection to detour around its upper endpoint.

The [problem](problems/observed-order.json) and
[template](observed-order-template.json) are unchanged copies of the first tested
geometry. Both orders use the same cold native project, 0.2 mm trace width,
0.2 mm copper/edge clearance, 0.25 mm routing grid, 100,000 expansion limit per
attempt, and no rip-up. The source has two opens and zero ERC, design DRC,
schematic-parity or metadata findings.

| Cold order | Admitted result | Remaining opens |
| --- | --- | --- |
| [Horizontal, vertical](observed-order-horizontal-first.json) | 1/2 nets; vertical is grid-disconnected | 1 |
| [Vertical, horizontal](observed-order-vertical-first.json) | 2/2 nets; zero native findings | 0 |

The adaptive [off](observed-order-adaptive-off.json) and
[on](observed-order-adaptive-on.json) configurations differ only in
`retry_observed_failures_before_forecasts`. Under identical two-pass limits, both
automatically choose vertical-first on their second pass and finish with the
same copper, configurations and search work. Enabling the flag uses the observed
failure directly and skips independent forecasts; the disabled arm spends
0.862 seconds on forecasts. This establishes the mechanism, not a completion
gain on harder boards or a broad timing improvement. There is no Freerouting
comparison for this control.

## Reproduction

Run from the repository root with a release executable and `kicad-cli` available.
Use a new output directory for each reproduction. Native verification creates
combined copper previews automatically.

```sh
order_binary=target/release/pcb-maker
order_case=benchmarks/competitive/mechanisms
order_output=build/observed-order-reproduction

"$order_binary" generate-semantic-kicad-ladder \
  "$order_case/problems/observed-order.json" declared \
  "$order_case/observed-order-template.json" "$order_output/generated"
"$order_binary" materialize-kicad-rung \
  "$order_output/generated/declaration.json" 2 "$order_output/native"
"$order_binary" repair-kicad-silkscreen \
  "$order_output/native/02-v" observed-order "$order_output/source"
```

The last command returns 1 because the two intended opens remain. Inspect
`source/verification.json`: `erc_violations`, `drc_design_violations`,
`schematic_parity_issues` and `library_metadata_warnings` must all be zero;
`selected_net_unconnected_items` must be 2. Stop if those checks differ. The
cleanup changes silkscreen annotations, not the routing problem.

Run each command separately: horizontal-first is expected to return 1 after
retaining its admitted first route; vertical-first returns 0.

```sh
timeout 120s "$order_binary" route-kicad-board-sequential \
  "$order_output/source" observed-order "$order_output/horizontal-first" \
  "$order_case/observed-order-horizontal-first.json"
timeout 120s "$order_binary" route-kicad-board-sequential \
  "$order_output/source" observed-order "$order_output/vertical-first" \
  "$order_case/observed-order-vertical-first.json"
```

To test automatic order recovery, both commands below should return 0. Each
starts cold from the same source and has two passes and a 120-second wall cap.

```sh
timeout 120s "$order_binary" route-kicad-board-adaptive \
  "$order_output/source" observed-order "$order_output/adaptive-off" \
  "$order_case/observed-order-adaptive-off.json"
timeout 120s "$order_binary" route-kicad-board-adaptive \
  "$order_output/source" observed-order "$order_output/adaptive-on" \
  "$order_case/observed-order-adaptive-on.json"
```

## Retained evidence

The original run retains its
[source hashes and verification](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/source-inputs.json),
[frozen geometry/policy](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/plan.json),
[sequential outcomes](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/order-processes.json),
[blocked render](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/horizontal-first/result/preview.svg),
and [successful render](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/vertical-first/result/preview.svg).
The sequential executable SHA-256 was
`5cf9a77a089e790662b8059daa937483ba0fffeda3755626fd43925c93a95e1c`.

The [adaptive comparison](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/comparison.json)
and [audit](../../../build/whole-board-reset-2026-09-08/global-routing/observed-order-fixture/audit.json)
use executable SHA-256
`dd6c5e5f50738707ce59e9e996c9e7a68121087035fa58d5c8ad844c995f4ec5`.
The retained JSON inputs here match those frozen experiment inputs byte for byte;
regeneration with another KiCad version may change native serialization/hashes.

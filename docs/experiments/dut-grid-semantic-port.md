# Semantic DUT grid-router port

## Question

Can the bounded A* kernel from `testing-esp32-duts` operate behind the durable
`layout-trace` problem/candidate boundary without losing layers, vias,
multi-terminal connectivity, exact validation, or failed-search evidence?

This is an integration experiment, not a verdict on the DUT routing approach.
Most higher-level policies from that router have not been ported yet.

## Selected design

- `pcb-grid-router::DutAStar` owns only bounded grid search. It knows nothing
  about components, pads, nets, or JSON.
- `pcb-routing` compiles semantic geometry into a grid request and converts a
  found path into the shared candidate route graph.
- Legacy two-terminal branches retain their IDs. Modern multi-terminal nets
  receive a canonical Euclidean Prim tree, so input permutation does not alter
  branch identity.
- Each terminal-layer combination is searched explicitly. Layer changes become
  durable via decorators.
- Partial candidates, `no_path`, and `budget_exhausted` are serialized. No-path
  is not conflated with a work-budget limit.
- A separate bounded negotiated policy makes earlier routes costly rather than
  forbidden, maps exact copper conflicts back to history states, and retains
  every pass. The hard-reservation policy remains available for comparison.
- Every result is independently checked as continuous geometry and explicit
  electrical connectivity. A grid path is never itself a correctness claim.

Two raster safety modes are retained for controlled comparison. The default
`exact_centerline` matches the predecessor's centerline-grid intent and relies
on exact DRC after materialization. `conservative_cells` reserves another half
cell diagonal so any segment incident to a legal node is clear, at the cost of
closing narrow channels. A poor result in either mode is evidence to inspect,
not grounds to delete the other mode.

## Current observations

Measurements use deterministic work rather than wall-clock time. The adaptive
configuration tries 0.5, 0.25, then 0.1 mm only after `no_path`; it does not
refine a budget-exhausted search.

| Fixture | Result | Searches | Expansions | Vias | Exact gate |
| --- | ---: | ---: | ---: | ---: | ---: |
| `forced-crossing-two-layer.json` | 2/2 branches | 2 | 10,672 | 2 | pass |
| Same crossing, negotiated default | 2/2 branches | 2 | 13,536 | 2 | pass |
| `multi-terminal-tree.json` | 2/2 branches | 2 | 148 | 0 | pass |
| `placement-perturbation-unblocks-routing.json`, random seed 0, 0.5 mm only | 0/1 branches | 1 | 14,062 | 0 | fail closed |
| Same random pose, adaptive grids | 1/1 branches | 3 | 194,049 | 0 | pass |
| Same fixture, harmonic pose, adaptive grids | 0/1 branches | 3 | 432,136 | 0 | fail closed |
| Harmonic pose plus blocked-frontier axis repair | 1/1 branches | 1 selected repair | 4,325 selected-route expansions | 0 | pass |

The first two cases establish that the adapter can express a layer-changing
crossing and a complete explicit three-terminal tree. They say very little
about scale or route quality.

The perturbation case exposed two different mechanisms. At random seed 0 the
legalizer moves `WALL` to y=15.6 mm, leaving an exact but grid-sensitive channel.
The 0.5 and 0.25 mm grids report `no_path`; 0.1 mm finds a 341-point route after
124,059 expansions and passes exact DRC. The original unfavorable result was
therefore grid aliasing, not evidence against the approach. Coarse-to-fine
retry now preserves all three outcomes and the blocker identities.

The harmonic initializer retains `WALL` at y=15.0 mm because it is not connected
to a net. Even the 0.1 mm search has no channel there. Blocked-frontier evidence
identifies `WALL` as the only movable semantic blocker. The first coordinator
now samples that component's declared vertical degree of freedom in isolated
transactions. Its first proposal moves the wall by -0.8 mm, routes on the
coarse 0.5 mm grid in 4,325 expansions, passes both exact gates, and becomes
the selected candidate. The initial 432,136-expansion failure remains serialized
beside it and can be opened in the viewer.

This establishes a real tracer-to-placer mutation, but only as a baseline. It
does not yet derive a force, move several coupled blockers, retain alternative
complete candidates, or operate continuously.

## Reproduce

```sh
cargo run -- route-grid \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  run/forced-crossing.route.json \
  experiments/configs/dut-grid-default.json

cargo run -- route-grid-negotiated \
  benchmarks/imported/layout-trace/forced-crossing-two-layer.json \
  run/forced-crossing-negotiated.json \
  experiments/configs/dut-grid-negotiated.json

cargo run -- place-route-grid \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  random run/perturbation.route.json \
  experiments/configs/dut-grid-adaptive.json

cargo run -- view-route \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  run/perturbation.route.json run/perturbation.html

cargo run --release -- repair-route-grid \
  benchmarks/imported/layout-trace/placement-perturbation-unblocks-routing.json \
  harmonic run/perturbation-repaired.json \
  experiments/configs/dut-grid-adaptive.json \
  experiments/configs/blocker-axis-repair.json
```

Substitute `harmonic` for `random` in the third command to reproduce the
unrepaired router-to-placer failure. That command exits unsuccessfully after
writing its evidence; the fifth command applies the repair policy to the same
starting pose.

## What this does not yet establish

- Negotiated congestion is present as a bounded alternative, but selective
  rip-up is not. Negotiation cannot repair static obstacle/terminal escape
  failures and can oscillate before removing every exact copper conflict.
- Multi-terminal nets route fixed Prim edges rather than attaching each new
  terminal to the existing copper tree as the mature DUT router does.
- Corridor guides, waypoints, completion passes, topology shake, bus lanes,
  clearance repair, and failure-neighborhood extraction are not ported.
- Continuous edges are checked during A* and invalid directional transitions
  are learned under a retry bound. Negotiated inter-route conflicts are instead
  admitted provisionally and fed into the next pass; exact validation remains
  authoritative.
- The router-to-placer coordinator currently samples one blocker's declared
  axes. There is no continuous pressure/correction field, multi-component
  repair, GPU search, or full-board result.

## Next discriminating experiment

Compare the discrete axis-sampling baseline with a direction derived from the
blocked frontier's spatial distribution, then with a field/pressure correction.
Use identical initial poses, grid portfolios, and work budgets, and retain the
first divergent proposal in the viewer. A second fixture with two coupled
movable blockers is required before generalizing from this one-wall success.

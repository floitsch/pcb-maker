# Orthogonal conflict-action baseline

This experiment starts from the declared straight seed routes in
`benchmarks/small/conflict-action-orthogonal-crossing.json`. The seed is
electrically complete and has exactly one exhaustive `trace_trace_clearance`
finding between a west-east branch and a north-south branch.

The coordinator retains the six requested depth-one actions as independent
candidate states:

| Action | Trace length | Vias | Bends | Cost at 5 mm/via | Exact |
| --- | ---: | ---: | ---: | ---: | --- |
| A around B north | 64.897301 mm | 0 | 2 | 64.897301 | pass |
| A around B south | 64.897301 mm | 0 | 2 | 64.897301 | pass |
| B around A west | 77.146081 mm | 0 | 2 | 77.146081 | pass |
| B around A east | 77.146081 mm | 0 | 2 | 77.146081 | pass |
| Via pair on A | 54.000000 mm | 2 | 0 | 64.000000 | pass |
| Via pair on B | 54.000000 mm | 2 | 0 | 64.000000 | pass |

The default configuration therefore selects the via pair on A by deterministic
tie-break. Raising the penalty to 20 mm per via selects A-around-B north at
64.897301 cost-mm. Exactness does not change: all six states pass in both
controls. This demonstrates that route length plus a fixed per-via cost drives
selection rather than action enumeration order.

Run and inspect the complete frontier with:

```sh
cargo run -- explore-conflict-actions \
  benchmarks/small/conflict-action-orthogonal-crossing.json \
  run/conflict-actions.json \
  experiments/configs/conflict-action-default.json
cargo run -- view-route \
  benchmarks/small/conflict-action-orthogonal-crossing.json \
  run/conflict-actions.json run/conflict-actions.html
```

The result retains the invalid source assessment, every candidate, exact
assessment, action identity, parent state, depth, route length, via/bend count,
and `path + heuristic` score. The existing route viewer plays the invalid
crossed seed as frame zero followed by all six exact-gated action states.

This depth-one control is not a general A* search. The current producer deliberately accepts
one pair of straight, orthogonal, same-layer traces. It emits analytic detours
around the blocking trace's endpoints and one alternate-layer span on either
branch. A separate bounded best-first experiment now provides recursive
expansion and candidate-fingerprint deduplication for two independent crossings.
Non-axis-aligned or interacting conflicts, obstacles, component motion, and
post-action push/pull shortening remain unimplemented.

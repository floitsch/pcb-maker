# Bounded best-first conflict-action baseline

This experiment starts from four electrically complete straight seed routes in
`benchmarks/small/conflict-action-two-crossings.json`. They form two independent,
exact `trace_trace_clearance` violations. The same replaceable six-action producer
used by the depth-one control may act on either unresolved crossing.

With `experiments/configs/conflict-action-best-first-default.json`, the bounded
search exhausts its producer-defined depth-two graph:

| Measure | Result |
| --- | ---: |
| Expanded states | 13 |
| Generated transitions | 84 |
| Unique retained states, including seed | 49 |
| Duplicate transitions | 36 |
| Exact-complete depth-two states | 36 |
| Selected path cost | 81.518 mm |

The 36 duplicates are the same final candidates reached by resolving the two
independent crossings in opposite orders. Candidate SHA-256 fingerprints make
those paths converge to one retained state. Every generated candidate is exact
validated before retention or deduplication, and every retained state includes
its parent, action history, score, candidate, and assessment.

Run and inspect the whole state graph with:

```sh
cargo run -- search-conflict-actions \
  benchmarks/small/conflict-action-two-crossings.json \
  run/conflict-action-best-first.json \
  experiments/configs/conflict-action-best-first-default.json
cargo run -- view-route \
  benchmarks/small/conflict-action-two-crossings.json \
  run/conflict-action-best-first.json \
  run/conflict-action-best-first.html
```

A depth-one control returns no exact solution and explicitly reports depth
truncation instead of claiming frontier exhaustion.

This is bounded best-first state search, but not a general router or an
optimality proof. The heuristic charges a configurable amount per remaining
exact violation and is not proven admissible. The current producer only handles
straight orthogonal same-layer conflicts; it does not yet repair conflicts
introduced by a non-straight earlier action, move components, call a geometric
router, or run push/pull shortening after an action.

The follow-up post-process evaluation adds a replaceable exact-gated optimizer
and an interacting shared-trace control; see
`experiments/conflict-action-post-process-evaluation.md`.

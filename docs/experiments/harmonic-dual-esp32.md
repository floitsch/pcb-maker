# Harmonic placement on the dual-ESP32 benchmark

## Question

Does the ported `layout-trace` harmonic-port initializer produce a feasible and
useful seed for the 43-component dual-ESP32 problem, and is its historical
connected-pair spacing floor helpful?

## Controlled runs

All successful rows used the same input, seed 0, semantic hard constraints, and
common exact oriented-body projector. `distance` is the initializer's weighted
pin-to-pin surrogate; no row claims routability.

| Policy | Result | Common sweeps | Constraint checks | Body-pair checks | Distance (mm) |
| --- | --- | ---: | ---: | ---: | ---: |
| Declared + projection | feasible | 3 | 24 | 2,709 | 503.390 → 496.297 |
| Grid | failed | — | — | — | `J_SOUTH` / `R_LED` overlap |
| Random | feasible | 53 | 424 | 47,859 | 503.390 → 720.375 |
| Connectivity barycentric | feasible | 43 | 344 | 38,829 | 503.390 → 375.417 |
| Harmonic, historical floor, long budget | feasible | 16 | 128 | 14,448 | 503.390 → 442.567 |
| Harmonic, floor disabled, long budget | feasible | 91 | 728 | 82,173 | 503.390 → 439.876 |

The common counters exclude work inside the harmonic proposal itself, so they
must not be read as total algorithm cost. That missing counter is a known
instrumentation gap.

## Investigation

The standard 64-sweep historical configuration stopped with `R_LED` and
`R_LINK_02` overlapping. Raising the harmonic legalization budget to 512
sweeps changed the blocker, disproving a simple fixed-obstacle explanation.

Inspection found two independent issues in the ported architecture:

1. The harmonic legalizer's connected-pair centerline spacing floor is more
   conservative than exact oriented-body non-overlap. It was incorrectly used
   as if it were validation law. It is now a serialized heuristic switch and
   exact validation uses body geometry only.
2. The common hard-constraint projector could pull components into overlap
   after harmonic legalization. Grid, random, and barycentric policies did not
   even run the harmonic-only overlap validator. Exact body separation now
   alternates with relational projection for every policy.

After those changes both long-budget harmonic variants are feasible. Disabling
the floor gives a slightly lower distance surrogate but substantially more
common projection work. This does not establish either variant as better for
routing.

## Decision

Keep both harmonic variants. Keep barycentric as a control: it currently has
the best connectivity-distance surrogate, but that is not copper or topology
evidence. Grid's newly visible failure is also retained rather than hidden by a
`feasible` flag.

The next comparison needs route generation and exact copper validation from
each placement, plus total proposal work. The viewer/evidence path must retain
failed poses and per-sweep dominant constraints; currently a failed placement
returns only the final blocker string, which is not enough to diagnose cycling.

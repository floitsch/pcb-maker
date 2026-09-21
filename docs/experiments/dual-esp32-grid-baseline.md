# Dual-ESP32 grid-routing baseline

## Scope

This is the first system-scale probe of the Rust semantic DUT adapter. It uses
the long harmonic placement, a 0.5 mm grid, two copper layers, no refinement,
and sequential hard reservation of earlier routes. It is a baseline for the
missing policies, not a claim that the board is solved.

## Results

| Route order | Routed branches | `no_path` | Expansions | Exact trace/obstacle findings |
| --- | ---: | ---: | ---: | ---: |
| width descending | 58/83 | 20 (`no_path`) + 5 exact-rejected | 3,888,191 | 0 |
| input | 61/83 | 18 (`no_path`) + 4 exact-rejected | 2,716,996 | 0 |
| longest first | 63/83 | 14 (`no_path`) + 6 exact-rejected | 2,711,879 | 0 |
| failure-directed reorder portfolio (best of initial + 8) | 65/83 | 14 (`no_path`) + 4 exact-rejected | 2,459,074 selected; 28,604,727 portfolio | 0 |
| selective rip-up, 24 attempts | 65/83 | 13 (`no_path`) + 5 exact-rejected | 16,867 final replacement; 8,062,958 total | 0 |
| negotiated congestion, 8 passes | 71/83 provisional | 5 (`no_path`) + 7 exact-rejected | 2,383,574 selected; 19,302,509 total | 7 trace/trace |
| negotiated congestion, 24 passes | 71/83 provisional | 5 (`no_path`) + 7 exact-rejected | 2,393,794 selected; 57,941,587 total | 1 trace/trace + 1 via/trace |

No branch exhausted its one-million-expansion search budget. The failed set
changes substantially with ordering; longest-first is five branches better,
and it strands a different collection of routes. This is direct evidence that
bounded reordering/rip-up or negotiated congestion is higher priority than
raising the A* budget.

Several failures stop after 1–41 expansions near dense ESP32/passive pads and
previous routes. Others explore 165k–369k states and hit the board boundary,
fixed pads, and keepouts. These are different mechanisms and should not share
one repair heuristic.

The first failure-directed policy promotes one failed branch ahead of up to
four routed-trace blockers from its frontier, then reroutes the whole candidate
transactionally. The best of eight proposals promotes `EAST_AUX_C_R` ahead of
`GND_E_38`, `PWR_W_3V3`, `CTRL_E_TX`, and `GND_W_1`, improving the exact result
from 58 to 65 routed branches. Other proposals range from 58 to 65. This
confirms that blocker identity is actionable, but 28.6M total expansions and
18 remaining failures also show why whole-board reorder is only a baseline for
selective rip-up/negotiation.

The selective policy freezes all legal copper outside a failed branch and up
to three routed blockers from its frontier. Accepted repairs become the next
baseline; rejected attempts remain serialized without mutating it. With 24
attempts the accepted chain is `0 -> 4 -> 11 -> 20` and reaches the same 65/83
as whole-board reorder. It uses 8.06M total expansions (72% fewer), and the
final four-branch replacement uses 16,867 expansions. This validates the
architecture and work reduction, but not completion: 18 failures remain.

The original center-node-only runs reported 65–67 routed branches but the exact
gate rejected 4–6 of those centerlines. A grid edge could clip an adjacent-pad
corner even though both endpoint cells were legal. The adapter now checks each
materialized continuous edge, bans the first invalid directional transition in
both directions, and resumes A* under a bounded retry count. Three
width-descending branches learn a valid alternative; the previously invalid
routes otherwise become honest `exact_geometry_rejected`/`no_path` outcomes.
Removing an invalid early route also changes later hard reservations, so the
headline routed count falls to 58–63 while geometric findings fall to zero.
That is stricter evidence, not an algorithm regression. `conservative_cells`
remains an available ablation, not a universal fix: it can close physically
valid narrow channels.

## Negotiated-congestion result

The negotiated policy is a separate bounded strategy; it does not replace the
hard-reservation router. Within a pass, earlier copper is assigned a soft cost.
After exact validation, trace/trace, via/trace, and via/via conflicts are mapped
back to the implicated grid states and receive persistent history cost. Every
pass and its partial candidate is retained. A provisional overlap is never
reported complete.

On the long harmonic pose, pass 0 routes 71 branches but has 67 exact copper
violations. The best eight-pass result has seven violations, and the best of 24
passes (pass 21) has two. This is substantial conflict repair and eight more
provisional routes than the longest-first hard baseline. It also oscillates:
the 24 pass conflict counts are not monotonic, and 57.9M expansions do not
remove the last conflicts.

More importantly, the same 12 branches fail on every pass: five `no_path` and
seven `exact_geometry_rejected`. Negotiation deliberately softens only prior
copper. It cannot open a channel blocked by board edges, component bodies,
pads, or invalid terminal escape geometry. The result therefore argues for
retaining negotiation and combining it with selective rip-up/static-escape
repair, not simply increasing the pass count or discarding the approach.

## Adapter performance finding

The first width-descending run took roughly two minutes despite only 2.37M A*
expansions. Profiling by inspection found that every grid cell scanned every
component obstacle, previous segment, and potential via conflict for every
branch. This was adapter work, not search work.

Rasterization now scatters each shape/segment only over its bounded cell range,
and constructs via legality as a combined plane mask. Before exact-edge
learning was enabled, the resulting JSON was byte-for-byte identical (SHA-256
`459c8019e7b07e67b7f74c032c8963f63799ab7d89b79ca43e781660415a4f5b`). A
warm release run completes in 2.0 seconds; the first recompiled run took 16.2
seconds. This optimization therefore changes cost, not routing semantics.

## What the board currently demonstrates

- The harmonic placer can produce a legal 43-component pose after its
  relational tolerance is interpreted consistently by the independent gate.
- The semantic adapter materializes 58–63 of 83 exact-geometric branches with stable graph,
  layer, via, failure-frontier, and exact-validation evidence.
- Sequential reservation is strongly order-sensitive and cannot complete the
  board.
- Negotiation raises the provisional routed count to 71 and repairs most
  inter-route conflicts, but it plateaus before exact completion and cannot
  repair the 12 static-geometry failures.
- Increasing the current search budget is not supported by the evidence.
- Exact edge rejection/retry removes optimistic centerline successes and
  records its own bounded work.
- The now-cheap probe is suitable for controlled route-order, rip-up, and
  negotiated-congestion experiments.

## Next experiments

1. Feed negotiated conflict groups into selective rip-up and compare combined
   candidates against each policy alone.
2. Separate terminal/static escape repair from inter-route congestion, then
   compare a safe portfolio of hard reservation and negotiation.
3. Compare the best DUT policy with a ported `layout-trace` corridor/visibility
   strategy on the same harmonic pose and deterministic work accounting.
4. Only then apply router-to-placer motion to system-scale blocker groups.

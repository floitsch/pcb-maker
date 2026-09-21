# Cut-capacity topology experiment

Run from the repository root:

```sh
python3 experiments/topology-routing/run.py
```

The script uses only the Python standard library. It automatically writes an
SVG for **every evaluated board candidate**, complete candidate geometry and
clearance findings, deterministic summary/provenance JSON, and a standalone
animated `build/algorithm-exploration-2026-09-07/topology/viewer.html`.
The viewer embeds its images and data; it requires no server or adjacent files.
Playback shows search attempts, not a physical motion simulation or CPU timing.

## What this tests

A synthetic board has one vertical obstacle barrier with named horizontal
passages. A topology signature records each net's passage and the ordering of
nets within each passage. Fixed left/right terminal order is part of the model.
The electrical connection graph is identical across candidates; the signature
adds obstacle-relative route information that a connectivity graph lacks.

Demand across a passage is the sum of trace widths plus pair clearance and
clearance to both walls. Thus a topology model **can carry width constraints**;
it need not discard them. Width is a necessary capacity constraint here, and
does not by itself certify realizability. A separate realizer assigns lanes and
fanout segments. An independent validator checks continuous Euclidean distances
between all different-net segments and between trace capsules and obstacles or
board edges, and checks the terminal endpoints. The validator never reads the
graph's capacity or ordering conclusions.

Blind enumeration tries assignments in increasing Hamming distance from the
same initial proposal. Conflict-guided search moves only nets consuming an
overloaded passage, or swaps the nets causing a corridor-order inversion. It
prioritizes candidates by remaining capacity/order conflict, then edit count.
If the cheap graph checks pass but geometry fails, it retains a bounded complete
fallback over the tiny assignment family. Failed geometry is never accepted.

Both policies materialize the whole small assignment universe for enumeration
and fallback. This is a prototype, not a scalable search implementation. Guided
priority computations are explicitly counted; a reduction in realization calls
is **not** automatically a runtime reduction. Every trial is still realized and
rendered, even if the graph could have rejected it cheaply.

## Results on 2026-09-07

| Fixture | Blind geometries | Guided geometries | Blind / guided graph checks | Result |
| --- | ---: | ---: | ---: | --- |
| Two traces, 0.37 mm gap | 2 | 2 | 2 / 5 | One trace changes passage |
| Six nets, capacity 1 + 2 + 3 | 277 | 5 | 277 / 42 | Same valid assignment |
| Capacity safe, wrong route order | 7 | 2 | 7 / 4 | Order conflict repaired |
| Capacity safe, first realization fails | 2 | 2 | 2 / 6 | Validator rejects; alternate passage works |
| Mixed 0.15 / 0.45 mm widths | 12 | 4 | 12 / 14 | Width demand directs redistribution |
| Three nets, only two unit-capacity passages | 8 | 7 | 8 / 17 | No valid result in tested family |

The 0.37 mm case explicitly validates the initial geometry as zero-width,
zero-clearance paths, then rejects it using real 0.15 mm traces and 0.10 mm
clearance. Two traces need `2 × 0.15 + 3 × 0.10 = 0.60 mm` at this cut.
One trace needs `0.15 + 2 × 0.10 = 0.35 mm` and fits.

The capacity-safe realization control has exactly enough passage width but
fanout segments approach at an angle. Their perpendicular clearance becomes
slightly less than the nominal parallel-lane spacing. This demonstrates why
the continuous geometry gate remains necessary. It does **not** prove that
another geometry in the rejected topology could not work.

The last control has a simple cut certificate: each of the two passages holds
at most one of the three required traces. This proves insufficiency only for
the two modeled passages, fixed obstacles, and single layer. Guided search
evaluates seven of the eight assignments, so its exhausted queue alone is not
an exhaustive impossibility proof. The blind control independently enumerates
all eight. No claim extends to moving components, different board boundaries,
vias, or arbitrary detours.

### Candidate-order sensitivity

The 277-versus-5 count depends on candidate ordering. A further twelve matched
pairs keep the same six-net physical board and use four initial assignments
(all middle, all upper, all lower, and a mixed order) crossed with three
deterministic permutations of net/corridor labels (seeds 11, 29, and 73).
Both policies get exactly the same initial assignment and label order per pair.
The seeds were fixed before running the sweep.

| Metric across twelve runs | Blind | Conflict-guided |
| --- | ---: | ---: |
| Evaluated geometries, total | 3,508 | 79 |
| Evaluated geometries, minimum–maximum | 76–548 | 5–10 |
| Evaluated geometries, median | 284.5 | 5.5 |
| All graph checks, including priority work | 3,508 | 521 |
| Clearance-valid solutions | 12 / 12 | 12 / 12 |

The guided policy uses fewer evaluated geometries in all twelve pairs, but the
range confirms that a single blind count is not robust. These twelve runs are
different search conditions on **one physical board**, not twelve independent
hardware benchmarks. `sensitivity.json` retains every per-case count and label
permutation; `sensitivity.html` animates every retained candidate. These counts
do not compare against a strong industrial router or establish runtime speed.

No accepted candidate relies on raster sampling. All candidate signatures are
unique within a run. The test also checks basic segment crossing, parallel
distance, and disjoint-collinear distance cases. These results establish that
the experiment's assertions hold; they do not establish industrial DRC coverage.
There is no KiCad verification and no ESP32 board in this experiment.

## Implication for the program

This is worth pursuing as a small **routing feedback representation** before
attempting a new general topological router. Retain the obstacle pair defining
a bottleneck, its available width, the ordered routes consuming it, and their
width/clearance demand. An overloaded cut supplies a concrete choice: change
some consuming route's passage, or move one of the obstacles enough to open it.
Routes outside the conflict need not be perturbed initially.

The existing passage-capacity placement repair already measures a similar
quantity for one moving obstacle. This prototype adds multiple competing
traces and route-order information. A useful next test is to extract one real
failed ESP32 passage, propose a competing corridor allocation, and use the
existing native router/verification transaction to realize it. Preserve the
geometric router as the realization authority.

Changing an ordinary connectivity graph does not guarantee a new embedding.
Here named corridors and route order distinguish relevant choices only because
the obstacle arrangement is deliberately simple. General boards need richer
obstacle-homotopy and ordering signatures. Coarse capacities can miss diagonal
cuts, bends, pin escape, and interactions between adjacent passages. Vias also
consume local geometric area; treating them as free cross-layer graph edges
would invalidate this model.

## Primary sources and prior project work

Yu and Dai's 1997 paper explicitly represents variable wire widths and spacing
in a cut-based topological routing model and studies fast routability checks:
[Fast and Incremental Routability Check of A Topological Routing Using a Cut-based Encoding](https://tr.soe.ucsc.edu/sites/default/files/technical-reports/UCSC-CRL-97-07.pdf).
The prototype implements a much smaller special case, not their full algorithm
or routability theorem.

Negotiating overused routing resources has an established parallel in
[McMurchie and Ebeling's PathFinder](https://janders.eecg.utoronto.ca/1387_2015/readings/pathfinder.pdf).
PathFinder uses congestion costs on routing resources; this prototype instead
uses explicit capacity conflicts in a small best-first discrete search.

The prior program experiment is
[Pressure-directed blocker repair](../../docs/experiments/pressure-directed-blocker-repair.md),
which already found that measuring passage deficit can be more useful than a
generic blocked-frontier pressure vector for deciding component displacement.

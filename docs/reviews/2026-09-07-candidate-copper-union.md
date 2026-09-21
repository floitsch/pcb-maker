# Candidate scoring must count overlapping copper once

Comparing the gap-aligned candidate with its materialized native board exposed
a disagreement: candidate quality reported 8.050 mm, while native centerline
union reported 8.025 mm. The candidate runs from X=12 to X=20.025 and returns
to its terminal at X=20, all at Y=14 on F.Cu. Its last 0.025 mm overlaps copper
already present. Both measurements concerned the same valid route.

The shared route-graph normalizer skipped consecutive segments of one branch.
This correctly ignored ordinary bend vertices, but also skipped overlapping
backtracks. Native board statistics represented each stored segment as its own
branch, so their union calculation already handled this case correctly.

The normalizer now examines adjacent same-layer contacts and skips only those
that share a single existing vertex. Overlapping intervals are split at their
endpoints and deduplicated by the existing physical-edge materializer. This
fixes candidate quality and generated segment representation while retaining
logical branches, terminal positions and occupied copper. Ordinary bends and
copper on different layers retain their existing behavior.

## Evidence

On the identical retained candidate, corrected physical length is 8.025 mm,
stored candidate segment length remains 8.050 mm, and reported overlap is
0.025 mm. Applying it to a copy of the original native board passes native
verification. Independent interval union using KiCad's integer coordinates
confirms exactly equal occupied copper, grouped by net, layer, width and Y.
Placement and the original source files are unchanged. Every native check
retains its automatic rendering.

All 101 adapter library tests pass. New tests check the retained backtrack,
quality ranking against a slightly longer detour, ordinary bends, and separate
layers. An independent interval oracle checks 8,160 collinear walks across four
directions and both layers, including repeated vertices, backtracking, loops
and alternate branch partitions. The native gap regression now also checks
candidate length against materialized board length; all four boards pass.

A read-only audit remeasures 50 retained candidates from the completed
19-connection profiles and the gap-alignment experiments. Three lengths change:

| Retained candidate | Correction |
| --- | ---: |
| Open prefix 19, `SIG02_R_HEADER` | −0.007071 mm |
| Blocked prefix 19, `SIG02_R_HEADER` | −0.007071 mm |
| Focused aligned gap, `CROSS_ROW` | −0.025000 mm |

All candidate files remain unchanged. Winners in the four retained portfolios
with multiple candidates remain unchanged. The unit counterexample establishes
that the old double counting can reverse the ordering of two routes; this audit
does not claim that it changed those historical board selections. No new full
19-connection cold run was needed to validate this scoring correction.

[Validation](../../build/routing-improvements/candidate-copper-union-2026-09-07/validation.json),
[candidate audit](../../build/routing-improvements/candidate-copper-union-2026-09-07/retained-candidate-audit.json),
[exact native copper comparison](../../build/routing-improvements/candidate-copper-union-2026-09-07/physical-equivalence.json),
and [verified native rendering](../../build/routing-improvements/candidate-copper-union-2026-09-07/after/preview.png)
are retained.

# Boundary evidence finds actionable repairs missed by recent-history priority

On the same frozen 79-net `interf_u` board, with the same routing rules and
eight diagnosis trials, history priority finds **zero counterfactual routes**
for PC-RD. Boundary priority finds **seven**, of which **four** can be restored
to valid partial boards. The selected repair reroutes ENBBUF and PC-RD, reducing
native opens from 107 to 105 with no design/ERC/parity findings.

[Matched comparison and native copper views](../../build/interf-rd-boundary-2026-09-08/comparison/index.html).

## Why the two priorities differ

The most recent commits are the PC data/control bus connections. The first eight
history trials remove PC-IOW, PC-IOR and PC-DB7 through PC-DB2. None opens a full
route for PC-RD.

Inspection of failing branch 1 reconstructs the same earlier branch and routing
request used by the solver. Its start region reaches 62,906 grid states; the
finish region reaches 797,494. Sampling the smaller region's blocked boundary
ranks PC-A9, MD2, DIR, ENBBUF, PC-A2, PC-A10, PC-A1 and CLKLCA. This order is
computed from the actual prepared obstacles, without choosing net names by hand.
The first seven removals produce target routes. PC-A9, DIR, ENBBUF and PC-A1 also
produce admitted restored boards. The three other candidates fail restoration;
target connectivity alone is not credited as a repair.

[Reachable region and sampled blockers](../../build/interf-rd-boundary-2026-09-08/cut/index.html).

The boundary is sampled, not an exact minimum cut. The prepared request is the
first attachment search in that branch. The evidence does not cover every
possible branch order, tree attachment, alternative grid, or physical route.
Nevertheless, native restoration supplies a direct test of whether the suggested
changes are useful.

## Controls and reusable benchmark

`compare_yielding_priority.py` requires byte-identical PCB, project and schematic
inputs, identical non-priority configuration, equal diagnosis-trial caps, and
independent native repair audits. It records ranked counterfactual coverage,
restored-candidate counts, selected changed nets and combined copper views.
This comparison changes only `diagnosis.prioritize_boundary_blockers` in the
configuration and uses the same frozen executable as the whole-board run.

Both native repair audits pass. An additional readback comparison checks every
emitted segment and via of all four admitted candidates against the final route
receipts, including coordinates, layers, widths and drills. Source inputs,
footprint/pad poses, and unrelated copper are preserved. The selected board is
an isolated 80-net alternative, not a promoted sequential checkpoint.

This is evidence of better candidate selection, not a measured runtime speedup.
The runs share a machine with concurrent work; finding more target routes also
causes the repair tool to attempt more restorations. A* counts exclude boundary
flooding, heuristic preparation, native verification and rendering.

## Relationship to the whole-board run

The history-guided whole-board continuation also gets past PC-RD through the
larger pair-search mechanism: it displaces PC-IOW and PC-A9 and restores both.
Thus the comparison does not establish that boundary analysis is the only way
to progress. It demonstrates that geometric evidence can find useful single-net
changes before resorting to broader removal sets.

The whole-board continuation is terminal at 83/110 nets and 100 native open
items, with zero design/ERC/parity findings. It uses the original per-action
bounds and four available repair invocations. Its independent terminal audit
passes, preserving all 67 inherited commits and admitting 16 new ones. PROG-
remains blocked after eight single-net and sixteen pair
counterfactuals, all unsuccessful. A new continuation enables boundary priority
on this later failure and retains history-based pair recovery as a fallback,
without adding target-specific hints. That test is running; neither priority
should be declared universally superior from the completed PC-RD comparison.

The sequential failure message now reports compound-recovery failure when that
stage ran, rather than retaining only the preceding single-net failure message.
The existing 143 KiCad and eight benchmark tests pass. Both native comparison
runs use the earlier frozen binary; this reporting-only change does not alter
their geometry or search evidence.

One inspection limitation surfaced in the multi-terminal view: the selected
start can be an attachment point on an earlier provisional branch, while the
background shows only committed copper. A future diagnostic should draw that
prepared tree explicitly, distinguishing it from native committed routes. The
current report's first-attachment scope remains essential to interpretation.

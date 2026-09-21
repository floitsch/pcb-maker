# Reachable regions guide a successful restoration

The retained `interf_u` experiment now has a native-checked **66-net result with
123 opens**, compared with 65 nets / 125 opens before this investigation. No new
native design, ERC or schematic-parity findings appear. An independent comparison
against the original checkpoint preserves every other open item, all poses,
unrelated copper and the schematic/project bytes. Only MD2, PC-A6, PC-A7 and
PC-A2 change.

[Combined copper, reachable-region maps and audits](../../build/interf-restoration-demand-2026-09-08/index.html).

The final repair is now reproduced with automatic boundary-based removal
priority. The complete multi-stage recovery from the original checkpoint still
needs coordinator integration; the 66-net board is an audited experimental
artifact, not a newly committed sequential journal. The existing journal remains
at 65 nets. The board is incomplete.

## What the geometry revealed

The preceding experiment established that PC-A6 and PC-A7 repeatedly block each
other. A successful two-net counterfactual displaced PC-A6 and MD2; restoring MD2
left PC-A6 missing. Routing PC-A6 after yielding PC-A7 then blocked PC-A7 again.

This experiment reused PC-A7's counterfactual path as a soft forecast while
routing PC-A6. Strengths 1, 4 and 16 all changed the proposal but failed to
restore PC-A7 directly. Widths, clearances, placement, grid resolution and the
four-million A* expansion bound were unchanged.

The new reachable-region diagnostic explains more than the old branch failure:

| PC-A7 branch-zero request | States reachable from U2 | States reachable from BUS1 | Result |
| --- | ---: | ---: | --- |
| Ordinary reciprocal control | 9,756 | 15,158 | Disconnected |
| After demand strength 4 | 10,056 | 947,583 | Disconnected |

Demand opened BUS1's access to much of the board, while U2 remained in a small
isolated region. The map shows that region on both copper layers. Sampled
boundary evidence names PC-A4, PC-A11, PC-A6, PC-A2 and PC-A0, among other
obstacles. Fixed pads are reported separately from removable foreign copper.
This identifies candidate interventions; it does not establish a minimum cut.

Four manually selected older-net trials—PC-A4, PC-A11, PC-A2 and PC-A0—all permit
PC-A7 to route. PC-A4, PC-A2 and PC-A0 can then be restored under native checks;
PC-A11 cannot. The existing repair score selects PC-A2. Repeating with **no named
hints**, the new automatic policy tests PC-A4, PC-A11, PC-A6 and PC-A2. It selects
PC-A2 and produces byte-identical final native copper.

This result supports combining a route-changing proposal with fresh obstruction
analysis on the changed geometry. Neither the original removal list nor the
remaining terminal's name alone captured why the new proposal was still blocked.

## Program changes

`pcb-grid-router::reachable_states` exposes the full flood-fill map using the
same transition, via and diagonal-corner rules as the reachability gate. The
gate and diagnostic share their implementation; ordinary early termination is
preserved.

`inspect-kicad-routing-cut BOARD CONNECTION REPORT [CONFIG]` prepares an actual
production branch request and inspects its endpoint regions. Configuration has
`branch`, `maximum_samples_per_kind` and `routing`. Earlier branches may be
searched to prepare a shared tree. The inspected branch itself is not routed;
only the first attachment request is inspected when a ranked portfolio has
several. The report contains region runs, counts, sampled blocker geometry,
layers and indices within the prepared model. Those indices are not native
UUIDs. Model rules and source copper are never modified.

`experiments/whole-board/inspect_routing_cut.py` wraps that command with automatic
combined overview/local SVGs and a PNG. `--highlight-net` highlights that net's
sampled boundary geometry. It records source/binary hashes and retains a source
render even if inspection fails.

`prioritize_boundary_blockers: true` is an optional setting in both yielding
diagnosis and sequential rip-up configuration. After a typed grid-disconnection
failure, it inspects the failed branch, aggregates sampled hits for foreign
copper by net, and tries those nets before the existing hints and remaining
foreign nets. Fixed pads are excluded from removal priority. No net is dropped.
Inspection errors are retained and fall back to the declared order. Search
budget failures do not trigger this boundary policy. Defaults remain unchanged.

Diagnosis schema 5 retains the boundary report or its error. Native repair
audits independently reconstruct the priority order and automatically render the
boundary report. Compact routing summaries expose the region counts, actual
removal order and inspection errors.

## Validation and limits

- 142 KiCad adapter, 15 grid-router and eight benchmark tests pass. A fixed-wall
  control checks attribution and source preservation; a smaller sample budget
  preserves region membership and full boundary counts. Grid controls compare
  reachability with A* across small obstacle masks and verify that a full map
  continues beyond an early reachable finish.
- With boundary priority disabled, the frozen real-board diagnosis reproduces
  the previous candidate, errors and search work exactly.
- All three demand repair trees, the manually guided repair and the automatic
  repair pass independent native audits. The final result additionally passes
  a comparison against the original 65-net checkpoint, not only its provisional
  parent. Automatic and manual selected PCB files are byte-identical.
- A full boundary sample control preserves the exact region runs and counts.
  It changes the priority order. Default sampling inspects the 2,048 boundary
  transitions and 2,048 forbidden-via entries nearest the opposite primary
  endpoint; the full control covers all 6,108 planar boundary transitions and
  8,816 forbidden-via entries. Hit counts are advisory rankings, not independent
  causal tests or probabilities.
- Every native experiment retains combined copper renders. The diagnostic
  region and highlighted-copper PNGs were visually inspected. No new external
  router comparison, board-area result or full-board completion is claimed.

Next, make the coordinator retain and explore bounded compound recovery states:
route-changing demand proposals, displacement/restoration of more than one net,
and fresh boundary analysis after the geometry changes. Admission must compare
the entire compound result with its original committed parent. Use this measured
case as a regression rather than hard-coding its net names or exact sequence.

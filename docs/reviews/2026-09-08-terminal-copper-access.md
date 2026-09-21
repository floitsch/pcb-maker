# Recovering blocked terminal grid samples

The slow PIC placement exposed a routing failure unrelated to congestion: the
nearest grid point to JP1.2's anchor is blocked by JP1.1, although other points
inside the target pad have clearance. Making the grid finer does not reliably
fix this. The frozen four-net parent fails at both 0.5 and 0.125 mm; 0.25 mm
happens to sample a clear point.

The new production fallback routes this net at all three resolutions. Native
verification admits each insertion: opens decrease from 69 to 59, with the six
original silkscreen findings unchanged and no new physical/ERC/parity findings.
The entire 0.25 mm candidate equals the archived working candidate.

[Diagnostic crops and combined routing renders](../../build/terminal-access-2026-09-08/index.html).

## Diagnosis and implementation

`inspect-kicad-terminal-access` reports the pad geometry, permitted layers, local
grid centers, blocking objects/nets, outline clearance, full-width anchor
connector clearance and straight continuity within pad copper. Its Python
wrapper automatically renders the local geometry at each requested resolution.
These are geometric diagnostics, not native admission or path-search proofs.

| Grid | Nearest point | Clear points inside pad, connected to anchor |
| --- | --- | ---: |
| 0.5 mm | blocked | 4 |
| 0.25 mm | clear | 14 |
| 0.125 mm | blocked | 60 |

The anchor itself is only 0.45 mm from the adjacent VCC pad. A new 0.5 mm trace
with 0.25 mm clearance cannot start there. The existing target pad already
provides copper continuity to its clear contacts; drawing an additional trace
through the anchor is unnecessary and would introduce a violation.

When `FirstPadContact` cannot use its usual snapped terminal, the router now
enumerates unblocked grid points on that pad. Candidates must connect to the
anchor through a straight segment contained in actual pad copper. Boundary
intersections partition this segment into constant-membership intervals, so
endpoint membership alone cannot bridge a concave notch or a separate island.
The existing terminal trimming removes the virtual pad-internal connector;
only the physically clear routed portion becomes track copper.

The fallback respects permitted pad layers and plated connectivity, keeps all
obstacle masks and source rules, and scans at most 65,536 cells per layer.
Candidates are deterministically ordered. Existing valid snap cases and
`PadAnchor` behavior are unchanged. The router evidence suffix
`pad-copper-access-v1` records when recovery was used.

This is deliberately a conservative recovery: it does not search around
concave pad corners, recover anchors outside their copper, or guarantee that a
clear contact can reach the rest of the net. Native connectivity and design
rule gates still decide whether a generated route is admissible.

## Controls

All 134 KiCad adapter tests pass. Added tests cover disconnected islands,
concave notches, rotated rectangles, capsule boundaries and union seams;
permitted-layer and fully blocked contacts; and routing from a custom pad
whose anchor cannot accept a full-width trace. The synthetic routing control
confirms that the unsafe anchor is absent from the materialized path.

The real probes preserve the frozen parent's four routed nets, placement and
source dimensions. Each candidate receives native verification and automatic
combined front/back renders without component bodies. These remain incomplete
boards; routing one net is not whole-board or area credit.

Frozen executable SHA-256:
`c5d526a1473cc38b2a955cc657829396d781f110c41931125bb988529712c558`.
Inputs, source snapshot, diagnostics, probe logs and results are retained under
`build/terminal-access-2026-09-08/`.

## Whole-board result

The faster coupled placement also cold-routes all 34 PIC nets on the 0.5 mm
grid alone, in its first pass. The previous coupled-placement run needed its
0.25 mm alternative for VCC_PIC. Demand estimates are regenerated from the
cold placement, with the same source classes and dimensions. The result has
zero opens, no physical/ERC/parity findings, and the six original silkscreen
findings. Full-layout admission remains false and the driver exits 1 as
required by that annotation contract.

Independent sequence and adaptive-selection audits pass, including all 34
steps, preserved unrelated copper and poses, and native track/via dimensions.
All eight benchmark tests pass as well. The generated playback has 34 checked
frames; this run did not repeat a browser playback test. Combined copper
rendering was visually inspected.

[Whole-board result and playback](../../build/terminal-access-2026-09-08/pic-coarse/index.html).

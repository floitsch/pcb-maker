# Broader native placement geometry

The PIC placement pipeline previously stopped at C1: its courtyard is a
KiCad rectangle primitive, while the importer only accepted four separate
lines or one circle. Extending this exposed non-rectangular courtyards and
a component on the back side. The importer now reads all **63 footprints and
34 routable nets**, using their native courtyard geometry and original sides.

[Renderings and remaining failure](../../build/placement-coverage-2026-09-08/index.html) ·
[Geometry controls](../../build/placement-coverage-2026-09-08/controls-v3/index.html).

Follow-up: [legalization tracing](2026-09-08-placement-legalization-trace.md)
now shows that this failure was slow convergence. A larger explicit budget
reaches a native-checked PIC placement; six annotations remain and full routing
is underway. The measurements below describe the original 512-sweep experiment.

## Implementation

The placement model now accepts a union of convex body parts. The rectangular
envelope supplies bounds; it does not fill the spaces between those parts.
The importer triangulates simple straight courtyard loops, retaining P3's
socket cutouts, U3's spaces between leads and RV1's beveled boundary. Circles
and rectangle primitives keep their simpler representation. Curved/multiple
courtyard loops remain explicitly unsupported.

Body-side declarations distinguish front and back assembly. Body/pad and
pad/pad exclusions use matching layers, while through-hole pads participate
on both sides. Old models with no explicit side retain their previous
exclusion behavior. JP1 has a native back courtyard; no inferred body or
footprint replacement is used.

The spectral initializer partitions disconnected attraction blocks into
area-weighted seed regions. Those regions are starting positions, not new
placement constraints. PIC has one 57-component connected block and six
unconnected mounting holes. All receive fresh positions; input ordering and
original movable centers do not affect them. Fixed-component seeding remains
unsupported by this initializer.

Native translation audits now compare custom-pad primitives, anchor shapes and
zone-shape settings in addition to the existing pad inventory. The native
materializer retains those shapes and sides. The general runner also offers
`--materialize-only` for checking a proposed placement before routing.

## Evidence and limits

- Native-source membership checks compare 2,883 points against the three
  polygon decompositions, including their excluded envelope space.
- A fixed probe fits P3's notch, fails against a filled rectangular envelope,
  and fails inside actual same-side material. The opposite-side control passes.
- Custom-pad translation preserves the audited inventory; changing a polygon
  vertex is detected even though nominal pad dimensions and other fields match.
- The original PIC placement passes both independent and production geometry
  checks under explicit reference overhang allowances for its connector,
  socket lever and mounting-hole courtyards. Pads retain their edge constraints.
- The previously successful `complex_hierarchy` seed and accepted poses replay
  exactly. All 18 model, 41 placement, 130 KiCad and eight benchmark tests pass.
- The new `--materialize-only` path also passes the complete native translation
  and inventory audit on `complex_hierarchy`: 68 footprints, 165 pads, zero
  physical/ERC/parity findings. Its three annotation findings and 112 cold
  opens remain visible; routing was intentionally not run in this control.

The new cold PIC placement is **not successful yet**. With all bodies confined
to the board, legalization stops at C2/R10. Allowing the reference overhangs
removes that artificial constraint, but the 512-sweep run still stops at
C2/U5. Neither rejected seed earns native placement, routing or area credit.
The saved render is the initial seed, not the final stalled state.

The next diagnostic is to capture the legalizer's rejected poses and contact
history. That will distinguish a crowded component group, an oscillating
correction and a constraint-model mismatch before changing the search policy.

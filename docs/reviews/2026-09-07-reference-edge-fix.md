# Generated reference labels at board edges

The dual-ESP32 cold-prefix test stopped before routing: generated U1/U2
reference silkscreen crossed the board outline. The generator always placed
references above the component in local coordinates, without considering the
component rotation or the board boundary.

The template generator now transforms the preferred reference position into
board coordinates, clamps its text envelope inside the rectangular outline,
then transforms the adjusted position back into footprint coordinates. The
reference text stays horizontal and visible on front silkscreen. Footprint
poses and pad positions, sizes, shapes, layers, angles, and nets are unchanged
in the before/after native geometry comparison.

This is a bounded edge-placement repair for generated ASCII references. The
text envelope is conservative, not a general KiCad font metric. Native DRC
continues to check the result against pads, other silk, and the actual rules;
this does not implement a general silkscreen placement optimizer.

## Observed result

- Before: source board rejected with two `silk_edge_clearance` findings.
- After: source board admitted, followed by both requested connections routed
  autonomously from zero copper. Source and both routed boards have ERC 0,
  design DRC 0, parity 0, and no selected-net disconnects. The same 21 library
  metadata warnings remain under the existing explicit exemption.
- U1 text bounds move from x=17.9–19.7 mm to 19.5–21.3 mm; U2 from
  x=85.3–87.1 mm to 83.7–85.5 mm. The board spans x=19–86 mm.
- The second connection has zero vias and 44.001 mm of physical copper.

The integration test also contained a stale limit of 1,500 search expansions.
On the exact same second-connection routing input and configuration, the
unchanged release executable uses 10,429 and the repaired debug executable
uses 10,556. The test now retains a 20,000-expansion bound alongside its native
admission, length, via, shared-tree, and cold-start assertions. This is not a
routing speed improvement. A unique timestamp in the test directory prevents
PID reuse from colliding with retained results.

Retained [comparison](../../build/routing-improvements/dual-reference-edge-2026-09-07/comparison.json),
[cold-prefix manifest](../../build/routing-improvements/dual-reference-edge-2026-09-07/after/cold-prefix.json),
and [unchanged-router control](../../build/routing-improvements/dual-reference-edge-2026-09-07/old-router-control.json)
record the evidence. This establishes completion of the two-connection prefix,
not the full dual-ESP32 board.

Validation: all 86 native-adapter library tests and the existing two-connection
native integration test pass; formatting checks pass. The retained
[validation record](../../build/routing-improvements/dual-reference-edge-2026-09-07/validation.json)
includes the successful run's artifact directory.

Reproduce with:

```sh
cargo test -p pcb-kicad --lib --quiet
PCB_MAKER_KEEP_TEST_ARTIFACTS=1 cargo test --test kicad_ladder native_dual_esp32_cold_prefix_rebuilds_first_two_rungs_from_zero_copper -- --nocapture
```

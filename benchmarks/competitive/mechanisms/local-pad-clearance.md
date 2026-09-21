# Local pad-clearance routing regression

The native router previously applied only its configured clearance to foreign
pads. A mounting hole with a larger local override was therefore invisible as
a clearance constraint until native admission rejected the generated route.

[`tests/kicad_pad_clearance.rs`](../../../tests/kicad_pad_clearance.rs) generates
an opposite-layer terminal pair and a nearby 1.2 mm mounting hole. The hole's
pad requires 1.85 mm clearance; the routing portfolio uses 0.3 mm. The fixture
includes its footprint library and starts without any design-rule findings
other than the expected unrouted connection.

On byte-identical source PCBs and the same routing configuration:

| Result | Before | After |
| --- | ---: | ---: |
| Connections committed | 0/1 | 1/1 |
| Candidate native design findings | 1 | 0 |
| Physical track length | 20.141 mm | 20.970 mm |
| Vias | 1 | 1 |
| Search expansions | 82 | 3,288 |

The rejected candidate has native `hole_clearance`: required 1.850 mm, actual
1.050 mm. The repaired program's first candidate passes ERC, DRC, parity, and
connectivity with zero findings. Additional search work is expected here: the
old search ignored a real constraint. This is a capability/correctness result,
not a speedup or a Freerouting comparison.

The [comparison record](../../../build/routing-improvements/local-pad-clearance-2026-09-07/comparison.json)
contains the common source hash and full native assessments. Automatic previews
show the [rejected candidate](../../../build/routing-improvements/local-pad-clearance-2026-09-07/before/step-000-TOP_TO_BOTTOM/native-candidate-00/preview.svg)
and [accepted result](../../../build/routing-improvements/local-pad-clearance-2026-09-07/after/routing/result/preview.svg).

## Implementation and scope

The native model retains physical pad geometry and a separate local clearance.
It resolves the pad override first and otherwise inherits the footprint value.
An explicit pad zero takes precedence over the footprint, matching the
[KiCad 9+ pad-clearance semantics](https://docs.kicad.org/10.0/en/pcbnew/pcbnew.html).
The configured routing clearance remains a conservative floor.

The same resolved minimum is applied in point/edge rasterization, via checks,
shortening, local reroute diagnostics, and fixed-obstacle continuous-repair
constraints. Spatial-index bounds include the extra clearance, so distant
geometry cannot disappear from a query just because only its clearance region
intersects the proposed route. Physical copper bounds are unchanged.

This covers explicit nonnegative pad and footprint overrides with modern
zero semantics. It does not resolve project/netclass/custom design rules or
pair-specific exceptions. Unsupported negative/non-finite local values are
rejected. Native verification remains authoritative.

Unit regressions check override precedence, explicit zero, invalid values,
clearance regions spanning spatial cells, trace/via queries, shortening, and
preservation of physical bounds. Existing router and adapter suites pass
(9 + 88 tests), and both the new native clearance regression and the existing
native via-only keepout regression pass. Formatting checks pass.

```sh
cargo test -p pcb-kicad -p pcb-grid-router --lib --quiet
PCB_MAKER_KEEP_TEST_ARTIFACTS=1 cargo test --test kicad_pad_clearance -- --nocapture
```

# Complete PIC routing with original net classes

The PIC programmer now cold-routes all 34 nets while preserving its original
project net classes. Native verification reports zero ERC, design DRC, parity
and connectivity findings. This removes the POWER-class simplification from
the pcb-maker result. A subsequent [source-class Freerouting comparison](2026-09-08-source-class-comparison.md)
preserves these same basic geometry requirements and leaves one native open.
The earlier uniform-rule comparison remains a separate adapted case.

| Class | Connections | Track width | Clearance | Via / drill |
| --- | --- | ---: | ---: | ---: |
| POWER | GND, VCC | 0.8 mm | 0.28 mm | 1.6 / 0.6 mm |
| Default | Other named nets | 0.5 mm | 0.25 mm | 1.6 / 0.6 mm |

The complete result uses 1826.911 mm of physical track centerline and 18 vias.
The sequential process takes 196.252 seconds including native admission, with
2419688 A* expansions. These are one-run measurements, not a controlled speed
comparison with the earlier adapted-rule result.

## Rule handling and diagnosis

The old uniform configuration lost both the wider POWER tracks and its larger
clearance. Merely widening GND/VCC would miss half the problem: later Default
routes must retain the POWER clearance from already routed POWER copper.

`KiCadGridRouteConfig.connection_rules` is an optional complete map of resolved
connection IDs to track width, clearance, via diameter and drill. Grid routing
selects target geometry from the map. Foreign pad/track/via obstacles acquire
their own class clearance as a floor; the existing exact geometry and spatial
queries take the maximum with the target clearance and local pad/footprint
clearance. An incomplete map or invalid geometry fails explicitly.

The map is retained in every route candidate. Shortening, relaxation and via
action models also receive its foreign-net clearance floors. Empty maps preserve
the prior behavior and serialization. This is a compiled-rule interface; it
does not implement KiCad's pattern matching or a general custom-rule engine.

The source compiler asks pcbnew for each effective class, then emits the explicit
map with project/board hashes and class-assignment evidence. It rejects unresolved
composite classes and custom rule files rather than approximating them. All 111
named PIC nets resolve to the two expected classes. Trace widths here honor the
source class's specified routing dimensions; they should not be described as
universal minimum-width DRC rules.

## Verification

- All 34 accepted candidates carry the complete rule map and the correct target
  dimensions; every accepted step passes native admission.
- A native readback checks every track width and every via diameter/drill.
  GND has 91 stored segments, all 0.8 mm, and one via; VCC has 25 stored
  segments, all 0.8 mm, and no vias.
- Final project bytes and all native effective class assignments match the
  source. Component positions, rotations and sides match exactly.
- The final full native check passes. The upstream source files remain unchanged.
- With the feature disabled, all 34 adapted-rule candidates and resulting boards
  replay identically, followed by another clean native check.
- 108 KiCad and seven benchmark library tests pass. A geometric regression
  distinguishes a 0.26 mm physical gap, allowed by 0.25 mm but rejected by
  the foreign POWER class's 0.28 mm clearance. Exact segment checks and spatial
  queries agree. Missing rules and invalid dimensions are rejected.
- The uniform comparison harness's existing recursive geometry validation is
  now tested with mismatched per-net overrides. It rejects that unfair setup.

The frozen executable SHA-256 is
`2bbf51cacf796ccdc75b068fa3bbd5330cafe41ee31f5c94006d2233c5c8e185`.

## Reproduction and artifacts

Prepare a fresh copy of the pinned PIC source, leaving its project unchanged,
and remove copper with `strip-kicad-copper`. Compile source class assignments:

```sh
python3 experiments/whole-board/compile_net_classes.py \
  SOURCE/pic_programmer.kicad_pcb \
  benchmarks/real/pic-programmer/pcb-maker-sequential.json RULES
target/release/pcb-maker route-kicad-board-sequential \
  SOURCE pic_programmer RUN RULES/sequential-config.json
target/release/pcb-maker verify-kicad-rung RUN/result pic_programmer
```

The retained study's `manifest.json` records exact commands and process results.
`audit_source_classes.py` audits that study and builds its routing viewer.

- [Source-class routing playback and audit](../../build/pic-source-classes-2026-09-08/index.html).
- [Compiled native class evidence](../../build/pic-source-classes-2026-09-08/rules/net-class-evidence.json).
- [Disabled-feature replay](../../build/pic-classes-disabled-replay-2026-09-08/index.html).

The matching Freerouting export now passes the basic class and net/pin audit.
The placement-area track still needs faithful
body/overhang representation and explicit product constraints before shrinking
results can earn credit.

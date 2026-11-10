# KiCad's interf_u programmer board

The cold input for KiCad's `interf_u` demo (source `kicad-interf-u` in
`../sources.json`): 25 footprints, 110 routable nets, a non-convex T-shaped
outline, every component pose fixed. `prepare.sh` fetches nothing; run
`bash benchmarks/real/fetch.sh kicad-interf-u` first, then:

```sh
bash benchmarks/real/interf-u/prepare.sh target/release/pcb-maker OUTPUT/cold
```

It copies the fetched project, repairs the schematic's obsolete footprint
library links (`normalize-kicad-footprint-links`, report
`footprint-link-normalization.json`) and removes all inherited copper
(`strip-kicad-copper`, report `copper-strip.json`). The cold board can then be routed with
`route-kicad-board OUTPUT/cold interf_u <output> auto`.

The JSON files here (`pcb-maker-adaptive.json`,
`pcb-maker-sequential-*.json`, `grid-*.json`) are configurations of the
September 2026 research router, whose commands were removed; the
experiments they drove are written up in
[docs/experiments/interf-u-cold-baseline.md](../../../docs/experiments/interf-u-cold-baseline.md).
The project permits zero copper-to-edge clearance, which Freerouting's DSN
export cannot express exactly: an external comparison on this board is not
an equal-rule comparison.

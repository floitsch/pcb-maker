# Board corpus and results

`benchmarks/corpus/run.py` copies each board, strips every track, via and
copper zone, and then runs two modes:

- **Route** keeps the designer's placement and tests the router alone.
- **Place + route** re-places every footprint that is not mechanically fixed
  (locked, without nets, or on the board edge) and then routes, using the
  coupled loop from [placer.md](placer.md).

Rules are read from each board's `.kicad_pro`; nothing is configured per
board. A result counts as clean when native KiCad reports no unconnected items
and no DRC error other than silkscreen or library cosmetics, *beyond what the
stripped source already had* (for example Olimex's fiducials next to its
mounting holes), and the internal exact verifier reports nothing.

```sh
benchmarks/corpus/fetch.sh                      # downloads the KiCad demos
cargo build --release
python3 benchmarks/corpus/run.py build/corpus   # add --only NAME... or --skip-layout
```

## Results (2026-09-21, one thread)

Route times are routing only; place + route times are the whole coupled loop
including several routing probes and the final native check.

| Board | Reference copper | Route (designer placement) | Place + route (automatic) |
| --- | --- | --- | --- |
| ecc83 | 59 tracks, 0 vias, 1 zones | 9/9, 0 vias, 249 mm, 0.09 s — clean | 9/9, 0 vias, 230 mm, 6.5 s — clean |
| hierarchy | 364 tracks, 0 vias, 166 zones | 50/50, 0 vias, 1330 mm, 1.22 s — clean | 50/50, 0 vias, 1409 mm, 12.2 s — clean |
| pic | 370 tracks, 6 vias, 1 zones | 34/34, 1 vias, 1907 mm, 2.84 s — clean | 34/34, 1 vias, 1763 mm, 18.6 s — clean |
| interf-u | 731 tracks, 84 vias, 1 zones | 110/110, 58 vias, 4788 mm, 21.47 s — clean | 110/110, 51 vias, 4797 mm, 115.3 s — clean |
| olimex-c3 | 768 tracks, 88 vias, 160 zones | 34/34, 49 vias, 926 mm, 1.84 s — clean | 34/34, 58 vias, 918 mm, 19.2 s — clean |
| dut-c3 | 321 tracks, 57 vias, 2 zones | 42/42, 24 vias, 1391 mm, 3.15 s — clean | 42/42, 27 vias, 1231 mm, 22.4 s — clean |
| dut-c6 | 359 tracks, 68 vias, 2 zones | 42/42, 29 vias, 1310 mm, 3.61 s — clean | 42/42, 25 vias, 1281 mm, 15.0 s — clean |
| dut-s2 | 432 tracks, 83 vias, 2 zones | 46/46, 42 vias, 1473 mm, 4.15 s — clean | 46/46, 45 vias, 1612 mm, 24.5 s — clean |
| dut-s3 | 392 tracks, 64 vias, 2 zones | 46/46, 42 vias, 1384 mm, 4.35 s — clean | 46/46, 37 vias, 1377 mm, 21.3 s — clean |
| dut-esp32 | 345 tracks, 64 vias, 2 zones | 46/46, 38 vias, 1368 mm, 4.02 s — clean | 46/46, 32 vias, 1373 mm, 23.4 s — clean |
| sonde-xilinx | 208 tracks, 3 vias, 1 zones | 26/26, 2 vias, 688 mm, 0.68 s — clean | 26/26, 2 vias, 688 mm, 9.2 s — clean |
| multichannel | 576 tracks, 29 vias, 2 zones | 79/79, 24 vias, 2502 mm, 17.46 s — clean | 79/79, 33 vias, 2002 mm, 33.9 s — clean |
| stickhub | 1113 tracks, 87 vias, 5 zones | 42/45, 49 vias, 800 mm, 4.5 s — **4 unconnected** | **placement not legal** (dense two-sided board) |
| ngdevkit | 115 tracks, 29 vias, 2 zones (unfinished by its designer) | **timeout at 1800 s**, 38 of 175 nets still in conflict | not run |

Twelve of fourteen boards complete and are clean in both modes. The DUT boards
are the project's target workload (an agent or human names the parts and rough
constraints; the rest is automatic).

## What the failures say

- **StickHub** (16 x 40 mm, two-sided SMD, 0.15 mm rules) relies on five copper
  pours. Stripping them forces ground onto tracks, which takes the room the USB
  pairs need. Pours must be supported: keep the zones, route the other nets,
  let KiCad refill, and stitch what is still disconnected. The placer also
  fails here: it never changes a part's side and treats halos too generously
  for a board this dense.
- **ngdevkit** (174 x 134 mm, 186 footprints, four rule classes, about 1070
  pads; its designer never finished it) runs into the time limit: 31 iterations
  at about 56 s, with roughly 730,000 expansions per search. Once the
  present-congestion factor reaches its cap, contested nodes are walls while
  the search heuristic still assumes free-board cost, so searches flood. Next:
  cap the present factor and rely on history, bound the effort per search,
  route independent nets in parallel, and tighten the memory layout of the
  search.

## Growing the corpus

The KiCad demo repository also has four 4-layer boards (kit-dev-coldfire,
video, openair-max, tiny_tapeout), waiting for inner-layer support in the
adapter, and two very large ones (jetson-agx-thor, vme-wren: 1100-1500
footprints, 10-12 layers) as long-term targets. More open boards (Olimex and
others) can be added to `corpus.json`; boards whose source fails KiCad's checks
for other reasons than the baseline subtraction covers should be skipped.

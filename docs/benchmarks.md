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

## Results (2026-09-22, one thread)

Tracks and vias are stripped; copper pours stay (where copper is poured is a
design decision like the placement). `pours connect` means pads reached their
pour through stubs, vias and stitching; `pours tracks` means the pour nets were
routed as tracks because connecting through the pour left something open (the
router tries both and keeps the better board). Route times are routing only;
place + route times are the whole coupled loop including its routing probes
and the final native check.

| Board | Reference copper | Route (designer placement) | Place + route (automatic) |
| --- | --- | --- | --- |
| ecc83 | 59 tracks, 0 vias, 1 zones | 9/9, 0 vias, 160 mm, 0.09 s, pours connect — clean | 9/9, 0 vias, 140 mm, 6.4 s, pours connect — clean |
| hierarchy | 364 tracks, 0 vias, 166 zones | 50/50, 0 vias, 986 mm, 0.65 s, pours connect — clean | 50/50, 0 vias, 1119 mm, 11.0 s, pours connect — clean |
| pic | 370 tracks, 6 vias, 1 zones | 34/34, 0 vias, 1944 mm, 3.11 s, pours tracks — clean | 34/34, 6 vias, 2100 mm, 24.0 s, pours connect — clean |
| interf-u | 731 tracks, 84 vias, 1 zones | 110/110, 52 vias, 4610 mm, 11.16 s, pours tracks — clean | 110/110, 63 vias, 4674 mm, 386.7 s, pours tracks — clean |
| olimex-c3 | 768 tracks, 88 vias, 160 zones | 34/34, 50 vias, 904 mm, 1.53 s, pours tracks — clean | 34/34, 61 vias, 912 mm, 31.0 s, pours tracks — clean |
| dut-c3 | 321 tracks, 57 vias, 2 zones | 42/42, 29 vias, 1055 mm, 2.29 s, pours connect — clean | 42/42, 23 vias, 956 mm, 18.2 s, pours connect — clean |
| dut-c6 | 359 tracks, 68 vias, 2 zones | 42/42, 34 vias, 1031 mm, 3.42 s, pours connect — clean | 42/42, 24 vias, 1020 mm, 15.7 s, pours connect — clean |
| dut-s2 | 432 tracks, 83 vias, 2 zones | 46/46, 45 vias, 1211 mm, 3.88 s, pours connect — clean | 46/46, 51 vias, 1356 mm, 25.9 s, pours connect — clean |
| dut-s3 | 392 tracks, 64 vias, 2 zones | 46/46, 44 vias, 1129 mm, 3.38 s, pours connect — clean | 46/46, 42 vias, 1144 mm, 18.9 s, pours connect — clean |
| dut-esp32 | 345 tracks, 64 vias, 2 zones | 46/46, 41 vias, 1138 mm, 3.67 s, pours connect — clean | 46/46, 43 vias, 1446 mm, 32.8 s, pours tracks — clean |
| sonde-xilinx | 208 tracks, 3 vias, 1 zones | 26/26, 1 vias, 515 mm, 0.46 s, pours connect — clean | 26/26, 1 vias, 789 mm, 16.3 s, pours tracks — clean |
| multichannel | 576 tracks, 29 vias, 2 zones | 79/79, 25 vias, 2056 mm, 5.8 s, pours connect — clean | 79/79, 39 vias, 2077 mm, 56.4 s, pours tracks — starved_thermal×1 |
| stickhub | 1113 tracks, 87 vias, 5 zones | 43/45, 43 vias, 782 mm, 3.0 s, pours tracks — **9 unconnected** (17-pin +3.3 V) | **placement not legal** |
| ngdevkit | 115 tracks, 29 vias, 2 zones (designer never finished it) | 161/180, 1108 vias, 19 035 mm, 804 s, pours connect — **181 unconnected**, mostly the 207-pad ground and 90-pad 3.3 V pours | not run |

Twelve of fourteen boards are complete and clean in both modes. Compared with
the first run a day earlier, the router is 2x faster on the large boards
(two-level search: a coarse tile graph plans each connection and the lattice
search stays inside that corridor), fine-pitch pads that cannot hold a lattice
node get exact escape stubs, copper-layer text and graphics are obstacles, and
pour nets connect through their pours.

## What the failures say

- **StickHub** (16 x 40 mm, two-sided SMD, 0.15 mm rules): the 17-pin +3.3 V
  net does not fit once the other nets are in; its designer used a +3.3 V pour
  region plus a ground pour, which the router only handles for the largest
  pours. Smaller pours and better use of both sides are the next step; the
  placer also needs side assignment for a board this dense.
- **ngdevkit** (174 x 134 mm, 186 footprints, about 1070 pads; its designer
  never finished it): finishes in 13 minutes instead of hitting the 30-minute
  wall, with 161 of 180 nets. The 207-pad ground pour and the 90-pad 3.3 V pour
  are cut into 1500 pieces by the signal routing, and stitching cannot rejoin
  them all. Pour nets need to be planned first (keep pour continuity as a
  resource in the tile graph), not repaired afterwards.

## Growing the corpus

The KiCad demo repository also has four 4-layer boards (kit-dev-coldfire,
video, openair-max, tiny_tapeout), waiting for inner-layer support in the
adapter, and two very large ones (jetson-agx-thor, vme-wren: 1100-1500
footprints, 10-12 layers) as long-term targets. More open boards (Olimex and
others) can be added to `corpus.json`; boards whose source fails KiCad's checks
for other reasons than the baseline subtraction covers should be skipped.

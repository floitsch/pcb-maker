# Frozen real-board sources

The [whole-board reference suite](../../experiments/whole-board/README.md)
now keeps human layouts, complete cold-routing comparisons, unsupported cases
and the placement/area objective together. Start there for project-level
progress; the source and mechanism history below provides diagnostic detail.

This directory declares pinned, licensed upstream KiCad projects for the M0
small/medium-real corpus. Source projects are downloaded into ignored
`external/` directories and every archive and required input file is verified
before use.

```sh
bash benchmarks/real/fetch.sh kicad-ecc83-pp
```

The source schematic is the connectivity authority. Existing human tracks,
arcs, vias, and filled zones are reference output, never inherited by a cold
route-only run. A board is not executable in the competitive corpus until the
cold-source adapter can remove that copper while preserving footprints,
outlines, rules, graphics, rule-area keepouts, and schematic parity. Copper
zone declarations are routing decisions and are removed together with their
cached fills; plane-aware reconstruction remains a later milestone.

The first executable source is the KiCad ECC83 push-pull amplifier demo. Its
competitive rules match the source project's default net class exactly:
0.8 mm tracks, 0.4 mm clearance, and 1.2/0.6 mm vias.

```sh
cargo run --release -- benchmark-route-freerouting \
  benchmarks/real/ecc83-pp/freerouting.json \
  build/sequence-NNN-m0-real-ecc83-freerouting
```

The same fixture has a generic direct pcb-maker route manifest and a paired
comparison. Both competitors consume the one adapted zero-copper project; the
internal router does not require a board-specific semantic translation.

```sh
cargo run --release -- benchmark-route-compare \
  benchmarks/real/ecc83-pp/comparison.json \
  build/sequence-NNN-m0-ecc83-comparison
```

The adapter experiment, retained failures, admission semantics, and promoted
sequences 221--224 are documented in
[`docs/experiments/real-ecc83-cold-adapter.md`](../../docs/experiments/real-ecc83-cold-adapter.md).

The second executable source is KiCad's PIC programmer demo. The new
[source-class run](../../docs/reviews/2026-09-07-pic-source-net-classes.md)
completes all 34 nets with the original POWER/Default geometry and native checks.
Its present paired M0 manifest still flattens Default and POWER to uniform
Default geometry; a class-preserving external-router export has not yet been
admitted. That adapted comparison now completes all 34 nets with pcb-maker;
Freerouting leaves one native open. Earlier terminal-access and route-order
frontiers are retained as diagnostic history. Route-only admission keeps
per-step PCB DRC/parity/connectivity while avoiding redundant unchanged ERC
work. The earlier controls are
documented in
[`docs/experiments/pic-programmer-cold-baseline.md`](../../docs/experiments/pic-programmer-cold-baseline.md).

The Olimex ESP32-C3 source is structurally adaptable but not yet scoreable.
Sequence 231 proves that the current uniform DSN contract loses its 1.85 mm
mounting-hole pad clearances; the visually routed result has new native
violations. It is retained as a rule-capability preflight case, documented in
[`docs/experiments/olimex-c3-rule-preflight.md`](../../docs/experiments/olimex-c3-rule-preflight.md),
and is excluded from router solve-rate claims until those constraints lower
equivalently for both competitors.

The fourth pinned declaration, KiCad's `interf_u` programmer board, is the
third executable real source and first executable medium-real case. Its
non-convex T-shaped outline is lowered exactly rather than replaced by a
bounding rectangle. A reproducible preparation step repairs obsolete
schematic-to-project-local footprint library links, audits unchanged board,
copper, and placement hashes, and removes all inherited copper:

```sh
benchmarks/real/interf-u/prepare.sh \
  target/release/pcb-maker build/interf-u-cold
```

The ordered multiresolution cold frontier native-admits its first 16 of 110
routable nets. A 0.125 mm fallback is necessary for physically legal PGA pad
escape corridors that the conservative 0.5/0.25 mm raster closes. Ordered
fallback evaluation cuts A* expansions 87.5% and routed-step time 41.6% versus
exhaustively running both resolutions. Layer-aware obstacle broad-phase
rasterization preserves the final board byte-for-byte and reduces the routed
step time again to 94.104 seconds, 60.1% below the exhaustive control. The
source admission, negative controls, and sequences 240--246 are documented in
[`docs/experiments/interf-u-cold-baseline.md`](../../docs/experiments/interf-u-cold-baseline.md).

The fifth declaration is KiCad's hierarchical analog demo. It is the fourth
executable real source: hierarchy-aware footprint-link normalization repairs
both the root and reused child schematic, cold preparation leaves 112 expected
opens with no native findings, and a four-terminal one-net smoke reduces that
count to 109 with 14 segments and no vias.

```sh
benchmarks/real/complex-hierarchy/prepare.sh \
  target/release/pcb-maker build/complex-hierarchy-cold
```

The adapter failure, candidate-source rejections, and sequences 247--250 are
documented in
[`docs/experiments/complex-hierarchy-cold-baseline.md`](../../docs/experiments/complex-hierarchy-cold-baseline.md).

The first four entries were carried over from
`testing-esp32-duts/benchmarks/sources.json`. Their upstream commit, SPDX
license, archive hash, and per-file hashes are retained verbatim. The fifth was
selected from the same pinned upstream revision after structural and native
preflight. No upstream source file is redistributed by this repository.

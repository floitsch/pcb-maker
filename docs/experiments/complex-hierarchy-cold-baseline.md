# Complex-hierarchy cold baseline

Sequences 247--250 admit KiCad's `complex_hierarchy` demo as the fifth pinned
real declaration and fourth executable real source. It is a `small_real`
two-layer through-hole board with 68 footprints, 50 routable nets, 112 cold
native connection items, and an 8057.413 mm² rectangular outline.

## Why this source matters

Unlike the earlier flat schematics, this project instantiates a child sheet
twice. Its original connected board has 364 straight segments, no vias, 166
copper zones, and two copper graphics. The source itself passes current KiCad
ERC and DRC, but its root and child schematic use obsolete footprint-library
links. The first root-only normalization repaired 10 references and left 58
parity mismatches. That retained failure motivated hierarchy traversal rather
than a board-specific exception.

The normalization transaction now follows only referenced `Sheetfile` paths,
rejects absolute paths and paths escaping the project, and records before/after
hashes for every schematic. On a fresh pinned source it updates 10 root and 29
child-sheet symbols. The PCB SHA-256 remains
`d5f4162d4c766448832723d139f9e0fcb88a17c82340f10f6f6dc6c2042f0d69`,
and native verification then reports ERC=0, DRC=0, and parity=0.

## Cold preparation and smoke

The reproducible preparation is:

```sh
benchmarks/real/complex-hierarchy/prepare.sh \
  target/release/pcb-maker build/complex-hierarchy-cold
```

It removes all 364 tracks, all 166 copper zones, and both copper graphics. It
preserves the placement digest
`0a293721df0cd0417da208b340d5622ab9f7db3a8e017395638551ae9e88e8c6`
and exact board area. Native verification of the cold source has no ERC, DRC,
or parity findings and exactly the expected 112 open items.

The one-net smoke routes the four-terminal `-VAA` power tree at its declared
0.6 mm width. It uses 35,571 A* expansions, 14 segments, 87.769 mm, and no
vias. Native admission reduces the open-item count 112 -> 109 without adding
any ERC, DRC, or parity finding. Sequences 248 and 250 produce the same PCB
SHA-256, `cec2890b1a99b319b049bcb272acac936bbc3c8ca00bed3fac8cbdcaed5e1d46`.

Sequence 248 also exposed that a bounded sequential result retained the copied
source `verification.json` even after committing a natively admitted board.
Sequence 249 accidentally replayed with the old release binary while the new
one was still compiling. Sequence 250 validates the fix: candidate PCB plus
ERC, DRC, and verification reports are committed together, and the result
artifact itself reports 109 opens. These failures remain in the progress
journal rather than being hidden.

## Candidate survey

The same pinned KiCad revision was used to reject several tempting corpus
entries before declaration:

- `microwave` has no schematic connectivity authority;
- `cm5_minima` lacks a complete same-stem project triplet;
- `StickHub` is electrically clean but has a non-standard noncommercial
  license/exemption and a curved outline not yet supported by the direct
  router;
- `multichannel` starts with 36 ERC, 22 DRC, and 115 parity findings;
- `tiny_tapeout` is a four-layer future-capability case with 58 ERC and 621
  DRC findings in the current KiCad version;
- `RoyalBlue54L-Feather` is permissively licensed, but the pinned PCB contains
  malformed S-expression input and fails closed in the parser.

Sequences 251--252 extend that survey beyond the KiCad demo repository and
retain paired renders. Protocentral's Sensything CAP is a compact two-layer,
58-footprint/64-net design, but its pinned source reports 407 ERC, 77 DRC, and
one open item in the installed KiCad version. Hat Labs' Sailor Hat gateway is
an 85-footprint four-layer panel with multiple disconnected outline loops and
starts with 71 ERC, six DRC, and 31 parity findings. Neither is declared or
counted as executable; both remain useful future adapter/source-health cases.

The fifth declaration therefore broadens the real corpus without weakening
source health, licensing, or native acceptance rules. A full 50-net solve is
not claimed; the next M0 work is more independent small/medium sources, not a
long prefix on this board.

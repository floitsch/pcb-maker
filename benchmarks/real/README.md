# Pinned real-board sources

This directory declares pinned, licensed upstream KiCad projects (KiCad's
own demos and an Olimex board) and holds, under `external/` (gitignored),
every board fetched for benchmarking: these sources, the harvested GitHub
boards (`external/github`, [../github/README.md](../github/README.md)) and
their schematics (`external/github-sch`).

`sources.json` lists each source with its upstream commit, SPDX licence,
archive hash and per-file hashes. `fetch.sh` downloads one into
`external/<id>` and verifies every archive and required file before use:

```sh
bash benchmarks/real/fetch.sh kicad-ecc83-pp
```

The sources are `kicad-pic-programmer`, `olimex-esp32-c3-rev-c`,
`kicad-ecc83-pp`, `kicad-interf-u` and `kicad-complex-hierarchy`. No
upstream file is redistributed by this repository.

Two sources have a preparation step that turns the fetched project into a
cold input: it repairs obsolete schematic-to-footprint library links
(`normalize-kicad-footprint-links`) and removes all inherited copper
(`strip-kicad-copper`), keeping footprints, outline, rules, graphics and
rule-area keepouts:

```sh
benchmarks/real/interf-u/prepare.sh target/release/pcb-maker build/interf-u-cold
benchmarks/real/complex-hierarchy/prepare.sh target/release/pcb-maker build/complex-hierarchy-cold
```

The per-source JSON files (`freerouting.json`, `comparison.json`,
`pcb-maker-sequential*.json`, `placement-policy.json`, ...) and
`whole-board.json` are configurations of the September 2026 research
stack, whose commands were removed; they stay for the experiments under
`experiments/whole-board` and the write-ups in `docs/experiments/`. The
current Freerouting comparison script is `benchmarks/github/freerouting.py`.

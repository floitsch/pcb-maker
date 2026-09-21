# Dual-ESP32 Freerouting baseline

Sequence 193 tests the exact cold prefix-19 placement with a mature external
autorouter. This is a capability reference, not a dependency or a proposed
replacement for the research engine.

The source is regenerated from the same semantic prefix and placement policy
as sequence 188. It contains 43 components, 19 electrical connections (24
semantic route branches), and zero tracks, vias, or zones. KiCad 10 exports the
board to Specctra DSN; Freerouting 2.2.4 routes it headlessly; the resulting SES
is imported into the untouched source board and checked by KiCad again.

## Exact-rule result

The authoritative comparison uses the benchmark's declared geometry:

- trace width: 0.25 mm;
- clearance: 0.20 mm;
- via diameter/drill: 0.70/0.30 mm;
- two copper layers;
- identical fixed component placement;
- no inherited copper.

Freerouting completes all **24/24** branches in seven passes and 5.90 seconds.
The imported board has 161 KiCad segments, 16 vias, zero unconnected items,
and zero copper DRC findings. KiCad also reports 43 generated-footprint library
bookkeeping findings; these are not clearance or connectivity failures.

The paired source and result renders are copied to `build/progress/` as:

- `sequence-193-freerouting-prefix19-unrouted-{front,back}.png`;
- `sequence-193-freerouting-prefix19-exact-routed-{front,back}.png`.

The machine-readable measurements, checksums, DSN, SES, imported board, raw
DRC, statistics, and renders are retained under
`build/sequence-193-freerouting-prefix19-source/`.

## Fairness controls

KiCad initially exported its default 0.20 mm trace and 0.60 mm via rules. That
control reached 23/24 and stopped on `SIG03_W_R`. Correcting only the trace
width to 0.25 mm also reached 23/24 and stopped on `SIG06_W_R`. Correcting both
the trace and via geometry produced the complete result above. This sensitivity
is why only the exact-rule run is the capability baseline.

## Architectural conclusion

The benchmark and placement are routable without component movement. Our
current failure is therefore primarily a routing capability gap, not proof
that the placement is impossible. The important difference is not raw A*
budget: Freerouting makes repeated whole-board passes that rip up and replace
earlier decisions. Our current selective router generally freezes retained
copper and had only just begun bounded blocker-driven expansion.

The next conventional baseline work is consequently:

1. make selective reroutes connectivity-safe for terminal and internal vias;
2. preserve a provisional complete/incomplete board across repeatable passes;
3. use failed-route blockers to choose rip-up sets and alternative route order;
4. retain every pass plus exact validation evidence;
5. only after the conventional router is competitive, compare how continuous
   squeezing and component motion reduce the number of discrete repairs.

Freerouting is GPL-3.0 software. Its output and documented architecture are
useful reference evidence, but implementation code is not copied into this
MIT-licensed repository.

## Automated reproduction

Sequence 198 is the schema-current one-command reproduction:

```sh
cargo run --release -- benchmark-route-freerouting \
  benchmarks/competitive/dual-esp32-prefix19/freerouting.json \
  build/sequence-198-m0-freerouting-harness-prefix19
```

The runner regenerates the semantic prefix before placement, starts from zero
copper, applies the exact rules through KiCad's `pcbnew` API, exports DSN, runs
the hash-pinned Freerouting jar with bounded passes/threads/time, imports SES,
and runs the native KiCad gate. It records raw logs, tool versions, input and
artifact hashes, timings, statistics, and four source/result front/back
renders. Sequence 198 again completes 24/24 in seven passes with 161 segments,
16 vias, and no native connectivity, DRC, ERC, or parity finding. Freerouting
reports 4.71 CPU seconds; the router subprocess takes 9.27 seconds and the
complete regeneration/render/verification pipeline takes 19.68 seconds.

Sequences 193, 196, 197, and 198 all produce the same order-independent copper
geometry digest,
`57e9516c83184f3845e52af81dc54c6fb61b15e46d15d0c6f10b8b99460a5f47`.
Their raw board hashes differ because KiCad assigns new object UUIDs and may
serialize imported segments in a different order. Both hashes are retained:
raw hashes prove artifact identity, while the copper digest proves equivalent
routing decisions without treating volatile serialization as an algorithmic
difference.

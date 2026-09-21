# Dual-ESP32 growth smoke corpus

Sequence 210 is the first promoted corpus-level competitive run:

```sh
cargo run --release -- benchmark-route-corpus \
  benchmarks/competitive/dual-esp32-growth-smoke/corpus.json \
  build/sequence-210-m0-dual-esp32-growth-smoke-corpus
```

Every case independently prunes the semantic board before placement and starts
with zero copper. No placement or routing result is carried from one prefix to
the next. Each child runs pcb-maker and Freerouting with matching rules and
fixed placement, exact-gates both results, and retains its detailed report,
logs, artifacts, and front/back images.

| Prefix | Both exact-complete | Freerouting segments/vias/length | pcb-maker segments/vias/length |
| ---: | ---: | ---: | ---: |
| 0 | yes | 0 / 0 / 0.000 mm | 0 / 0 / 0.000 mm |
| 1 | yes | 3 / 0 / 21.824 mm | 5 / 0 / 22.081 mm |
| 2 | yes | 11 / 0 / 57.738 mm | 15 / 0 / 56.945 mm |
| 3 | yes | 14 / 0 / 84.383 mm | 22 / 2 / 87.234 mm |

The aggregate reports four finished cases, zero infrastructure/fairness
failures, four both-solved outcomes, 16 native-cache hits, no misses or
bypasses, and 68.293 seconds total elapsed time. Completion is aggregated
before quality, so the small copper differences do not obscure whether either
router fails a board.

The experiment exposed three pipeline bugs before passing:

- Sequence 206 showed that the benchmark schema prohibited prefix 0 despite
  the growth protocol requiring a connection-free first board.
- Sequence 207 reached Freerouting, which correctly reported zero initial and
  final unrouted items but emitted an empty SES. The runner now retains the
  cold zero-connection board only for this checked zero/zero case, then applies
  normal native KiCad admission.
- Sequence 208 showed that pcb-maker materialized target 0 and declared it
  complete without native admission. The solver now writes and requires an
  explicit zero-rung verification report. Sequence 209 then passed, but
  revealed that corpus child render names were overwriting earlier sequences.
  Sequence 210 includes the corpus sequence name and is append-only.

This is a functional growth smoke corpus, not completion of M0's corpus gate.
It contains four prefixes of one board family; mechanism fixtures, independent
small-real boards, licenses, and holdouts still need to be added.

## Physical metrics

Sequence 211 reruns the corpus after adding the common physical-copper metric
block to every board-statistics record and the aggregate. All four cases again
both-solve with matching placement/rules. The verifier fingerprint changed, so
the run correctly records 16 cold misses and no bypasses. Its 32 images are
retained under the sequence-211 prefix.

| Prefix | Router | Physical centerline | Track area | Via-annulus area | Used layers |
| ---: | --- | ---: | ---: | ---: | ---: |
| 0 | both | 0.000 mm | 0.000 mm² | 0.000 mm² | 0 |
| 1 | Freerouting | 21.824 mm | 5.456 mm² | 0.000 mm² | 1 |
| 1 | pcb-maker | 22.081 mm | 5.520 mm² | 0.000 mm² | 1 |
| 2 | Freerouting | 57.738 mm | 14.434 mm² | 0.000 mm² | 1 |
| 2 | pcb-maker | 56.945 mm | 14.236 mm² | 0.000 mm² | 1 |
| 3 | Freerouting | 84.383 mm | 21.096 mm² | 0.000 mm² | 1 |
| 3 | pcb-maker | 87.234 mm | 21.808 mm² | 1.257 mm² | 2 |

Every case has the same exactly measured 3,919.5 mm² rectangular board
outline. Straight-track centerlines are split at same-net contacts and
collinear overlap is counted once. Arc centerlines, refilled polygon area,
via annuli, drill area, and per-layer totals are also measured. The reported
area is deliberately a primitive sum rather than a planar union: it excludes
track end caps and does not subtract crossings or track/via/zone overlap. It
is diagnostic and is not used to trade completion for tiny area differences.

# Dual-ESP32 paired prefix-3 comparison

Sequence 201 is the first one-command run of pcb-maker and Freerouting from the
same cold semantic input:

```sh
cargo run --release -- benchmark-route-compare \
  benchmarks/competitive/dual-esp32-prefix03/comparison.json \
  build/sequence-201-m0-paired-prefix03
```

Both tools receive connectivity pruned to the first three connections before
the declared placement is regenerated. Both route from zero inherited copper
at 0.25 mm width, 0.20 mm clearance, and 0.70/0.30 mm vias. The comparison
rejects any pcb-maker routing entry with different geometry and checks that
the source and both results have the same component poses on the 0.0001 mm
Specctra exchange grid. Exact unrounded pose hashes remain in the report.

Both results pass native KiCad ERC, DRC, schematic parity, and selected-net
connectivity with no findings:

| Router | Complete | Stored segments | Vias | Stored segment length | Router/process time |
| --- | ---: | ---: | ---: | ---: | ---: |
| Freerouting 2.2.4 | 3/3 | 14 | 0 | 84.383 mm | 3.50 s |
| pcb-maker | 3/3 | 22 | 2 | 87.234 mm | 40.86 s |

The timing columns are diagnostic rather than a throughput verdict:
pcb-maker performs native admission for each incremental connection, while
Freerouting is imported and admitted after its whole-board pass. The common
runner takes 62.57 seconds sequentially, including regeneration, rendering,
and verification. The immediate competitive gap visible even on this small
case is repeated native-process overhead and avoidable layer changes, not
route completion.

Sequence 199 retained the first fairness-gate failure: hashes compared textual
coordinates and treated `43` and `43.0` differently. Sequence 200 fixed that
but exposed Specctra's coordinate rounding of four freely placed components by
at most 0.00005 mm. Sequence 201 retains exact and exchange-grid digests and
requires the source, Freerouting result, and pcb-maker result to agree.

## Native-verification cache experiment

Sequences 204 and 205 rerun the same paired comparison with the promoted
content-addressed native KiCad cache. Sequence 204 is cold for the new key
scheme and sequence 205 is warm:

| Run | Cache hits | Cache misses | Native verification time | pcb-maker process | Whole paired run |
| --- | ---: | ---: | ---: | ---: | ---: |
| 204, cold | 0 | 7 | 35.637 s | 38.824 s | 54.034 s |
| 205, warm | 7 | 0 | 0.900 s | 3.965 s | 19.170 s |

The warm run is 9.79x faster for the pcb-maker subprocess and 2.82x faster
end-to-end; native admission itself is 39.59x faster. Both runs have zero
cache bypasses, seven distinct keys, and produce the same final pcb-maker
board byte-for-byte (`6c08d47e7eb4210174028d95f568d06b33175b2836becae1d211cae91716925e`).
The board still exact-passes 3/3 with 22 segments, two vias, and 87.234 mm of
stored segment length. Front and back renders for both routers and both runs
are retained in `build/progress/`.

The key covers all local KiCad design inputs, routing-independent native
verification policy, zone-refill mode, exact `kicad-cli` version, and a digest
of the `pcb-kicad` implementation plus `Cargo.lock`. Generated reports are not
inputs. External library URIs bypass the cache, symlinked inputs are rejected,
and a corrupt immutable entry fails closed. This deliberately preserves hits
after changes confined to placement or routing crates, while conservatively
invalidating after changes to the KiCad integration crate or dependency graph.

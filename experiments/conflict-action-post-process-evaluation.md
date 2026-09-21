# Conflict-action post-process evaluation

The bounded conflict search now calls a replaceable processor after every
semantic action and before candidate fingerprinting, ranking, and queueing.
Processor errors are fail-soft: the unoptimized action state is retained with
the error rather than disappearing.

## Independent crossings

On `benchmarks/small/conflict-action-two-crossings.json`, all compared runs keep
the same exhaustive frontier: 13 expanded states, 84 transitions, 49 unique
states, 36 action-order duplicates, and 36 exact-complete solutions.

| Processor | Selected path cost | Exact solutions | Observation |
| --- | ---: | ---: | --- |
| none | 81.517674 mm | 36 | Control |
| exact line-of-sight point deletion | 81.517674 mm | 36 | No point could be deleted without restoring a clearance violation |
| exact vertex pull | 79.920588 mm | 36 | 1.597086 mm lower; 32 retained states improved |

The negative line-of-sight result is useful: the four-point analytic detours
are topologically necessary, but their vertices are not length-optimal. The
vertex-pull processor moves each interior point toward the midpoint of its
neighbors using bounded binary trials. A move is accepted only if exhaustive
validation preserves electrical completeness, introduces no new geometry
finding identity, and reduces total trace length.

## Shared-trace conflicts

`benchmarks/small/conflict-action-shared-trace.json` has one horizontal trace
crossing two vertical traces. Resolving a conflict by changing the shared trace
can make the remaining conflict non-straight and therefore unsupported by the
current six-action producer. The result retains these dead ends instead of
hiding them.

Both matched 20 mm/via runs expand 13 states, generate 48 transitions, retain
40 unique states, remove 9 duplicates, and find 12 exact-complete states. Eight
depth-one states record the producer's `not one straight via-free segment`
failure. The selected result changes as follows:

| Processor | Selected cost | Selected topology |
| --- | ---: | --- |
| none | 139.533434 mm | B west, then A north around C |
| exact vertex pull | 137.534271 mm | C east, then A north around B |

The 1.999163 mm overall improvement includes optimization after both actions;
the final action's retained evidence reports a 0.926068 mm saving from two
accepted vertex pulls. Both selected candidates pass exhaustive geometry and
electrical validation.

## Architectural conclusion

The post-action boundary is worthwhile: it can alter search ranking without
mixing optimization logic into action generation, and its effects and failures
remain inspectable. The current vertex pull is deliberately a CPU reference,
not the promised particle/GPU shortening pass. The existing small copper engine
projects clearance constraints but has no trace-length objective, so calling it
a shortener would be incorrect. A future engine processor should add an explicit
length/tension objective, component attachments, and component/trace contacts,
then use this same transaction and evidence contract.

Run the interacting comparison with:

```sh
cargo run -- search-conflict-actions \
  benchmarks/small/conflict-action-shared-trace.json \
  run/shared-trace-control.json \
  experiments/configs/conflict-action-shared-trace-default.json
cargo run -- search-conflict-actions \
  benchmarks/small/conflict-action-shared-trace.json \
  run/shared-trace-pulled.json \
  experiments/configs/conflict-action-shared-trace-vertex-pull.json
cargo run -- view-route \
  benchmarks/small/conflict-action-shared-trace.json \
  run/shared-trace-pulled.json run/shared-trace-pulled.html
```

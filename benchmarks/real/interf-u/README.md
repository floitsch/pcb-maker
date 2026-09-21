# Full-board native routing

`pcb-maker-adaptive.json` routes every discovered net from the fixed reference
placement. The earlier `pcb-maker-sequential-frontier.json` is a historical
16-net mechanism control; it is not a whole-board result.

The whole-board configuration uses ordered 0.5/0.125 mm routing, multi-source
tree attachment, independently generated demand and at most two full-board
passes. Four bounded single-net rip-up invocations use the fine grid. Native
source classes, edge clearance and hole spacing must be compiled into the
configuration by `route_placement_case.py`.

```sh
bash benchmarks/real/interf-u/prepare.sh target/release/pcb-maker OUTPUT/cold
python3 experiments/whole-board/route_placement_case.py \
  OUTPUT/cold interf_u benchmarks/real/interf-u/pcb-maker-adaptive.json \
  target/release/pcb-maker OUTPUT/routing --passes 2
```

The cold source has 25 footprints, 110 routable nets and 200 native open items.
Its original non-convex outline and every component pose remain fixed. Every
native verification automatically retains a combined copper rendering.

The September 8 run in `build/interf-whole-2026-09-08` uses the frozen
pre-completion-stopping executable. Its two-pass budget therefore remains
unchanged while newer executions stop after routing completion by default.

The native project permits zero copper-edge clearance. The current external
adapter rejects an exact translation because Freerouting's loaded outline has
positive half-width. A separately labelled unmodified-edge external control
retains 62 cold-input edge findings and finishes with five native opens and no
native design findings. It is **not an equal-rule comparison**. Keep this
exchange limitation separate from failures of pcb-maker's native-rule run.

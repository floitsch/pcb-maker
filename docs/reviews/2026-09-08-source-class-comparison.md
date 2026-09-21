# Source-class PIC comparison and session import settings

Both routers now have results on the same copper-stripped PIC programmer with
the original POWER/Default classes. pcb-maker completes all connections;
Freerouting 2.2.4 leaves one native open at the custom solder-jumper pad JP1.2.

| Router | Native opens | Design DRC findings | Track centerline | Vias |
| --- | ---: | ---: | ---: | ---: |
| pcb-maker | 0 | 0 | 1826.911 mm | 18 |
| Freerouting 2.2.4 | 1 | 0 | 2100.619 mm | 0 |

Both have zero ERC and parity findings. The incomplete external result does
not qualify for a route-quality win or loss; these figures describe one
development case, not general superiority. Freerouting took 7.790 seconds for
Java routing; pcb-maker's 196.252 seconds includes per-net native verification.
Those timing contracts do not support an algorithm speed ratio.

## Exchange audit

`experiments/whole-board/run_freerouting_classes.py` copies the terminal
pcb-maker study's cold source and exports directly, without flattening classes
or saving the source board. `audit_dsn_classes.py` checks all 111 named net/pin
inventories and four effective class dimensions: track width, clearance, via
diameter and drill. It verifies both circular via layer shapes and the drill
encoded in the via identifier. POWER remains 0.8/0.28 mm and Default remains
0.5/0.25 mm; both use 1.6/0.6 mm vias. The input has no inherited copper.

Native readback confirms unchanged class assignments, exact project bytes,
fixed placement at 100 nm and matching dimensions on every resulting copper
item. Four corrupted DSN controls (POWER width, clearance, missing class net,
and drill identifier) are rejected. This audit covers basic geometry and net
membership; it does not prove arbitrary rule or footprint-shape equivalence.

The remaining open is `/pic_sockets/VCC_PIC` at JP1 pad 2, (148.807, 97.790) mm.
Native close-ups show our route reaching its copper and Freerouting leaving
it isolated. The cause inside Freerouting or the geometry exchange remains
unproven.

## Production correction

The experiment caught `pcbnew.SaveBoard` migrating the project during SES
import, including a DRC severity default. The experiment retains that migrated
project as evidence and restores the original bytes before verification.
`pcb-benchmark` now also snapshots and restores project settings around session
import, including error paths and the initially absent-project case. This
preserves settings as they stood immediately before import; the existing
uniform-rule export is still an explicitly adapted benchmark.

All eight benchmark library tests pass. A fresh ECC83 Freerouting integration
run with executable SHA-256
`22bd7ec444ca657ae452deb71564ba7530c58914cd9799b189b2638362a4f06d`
completes with zero opens and unchanged source/result project bytes. Its two
design DRC findings are admitted against the existing source baseline.

## Artifacts and reproduction

- [Comparison, whole board and jumper close-ups](../../build/pic-source-class-comparison-2026-09-08/index.html)
- [Machine-readable comparison](../../build/pic-source-class-comparison-2026-09-08/report.json)
- [Negative controls](../../build/pic-source-class-freerouting-2026-09-08/negative-controls.json)
- [Production integration result](../../build/session-project-preservation-2026-09-08/run/competitive-result.json)
- [Our 34-step routing playback](../../build/pic-source-classes-2026-09-08/index.html)

With the pinned Freerouting jar and Java available as specified in the runner:

```sh
python3 experiments/whole-board/run_freerouting_classes.py \
  build/pic-source-classes-2026-09-08 build/pic-source-class-comparison-new
```

The comparison automatically renders source and result through native
verification. The retained local crops supplement those whole-board previews.
Browser playback was not tested. The next major placement milestone remains
a faithful body/overhang model that accepts the known human placement, followed
by a fully routed cold-placement candidate. No smaller routed board is claimed.

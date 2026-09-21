# Preserving room for later nets and learning routing order

The native sequential router now accepts optional forecasts of future copper
as soft spatial costs. Controlled experiments reduce the 70%-area ECC83 board
from **10 vias / 386.308 mm** to **3 vias / 345.004 mm**, and the PIC holdout
from **18 vias / 1826.911 mm** to **10 vias / 1826.835 mm**. Both selected boards
are complete under native ERC, design DRC, schematic parity and connectivity
checks. Physical rules and placement are preserved. ECC83 retains seven
separately classified library metadata warnings; PIC has none.

These are fixed-placement routing results on two boards. They do not establish
a universal routing policy, electrical-quality improvement, or a new result
against Freerouting. No new Freerouting comparison was run in this experiment.

- [Labelled copper comparisons, demand maps, and full result table](../../build/routing-demand-2026-09-08/index.html)
- [Independent native readback audit](../../build/routing-demand-2026-09-08/audit.json)
- [Directed pairwise interference matrix](../../build/routing-demand-2026-09-08/ecc83-orders/interference.html)

## Program change

`KiCadGridRouteConfig.routing_demand` is optional and defaults to absent. It
contains `strength`, `shoulder_mm`, and `alternatives`. Each alternative names
its electrical connection, weight, trace width, clearance, via diameter and
layered paths. Multiple alternatives can represent different embeddings.

The router deposits a full-cost clearance envelope with a linear shoulder.
It max-unions branches within an alternative so shared copper, duplicated
paths and extra vertices do not multiply pressure. Alternatives add according
to their weights. The current net never repels itself. The sequential driver
removes forecasts belonging to already committed nets. A via's advisory cost
accounts for its larger footprint and both copper layers. These costs affect
search preferences, never physical obstacles or native admission.

The kernel still uses the existing per-state entry-cost convention: planar
entry penalties are not scaled separately for diagonal distance. Very large
costs are capped below the forbidden-state sentinel; bounded integer search
and expansion limits still apply. This is an experimental cost model, not a
certificate of free passage capacity.

Shared-tree attachment selection must also consider router cost, or it can
prefer a shorter attachment that loses the spatial benefit. Experiments
therefore include a separate `router_cost` attachment control. Geometry-only
candidate quality and native completeness remain independent of forecast cost.

## Forecast protocol

`run_routing_demand.py` copies the full project, strips all source copper, and
independently routes every net against fixed pads and geometry. It tries two
via preferences, deduplicates identical path embeddings, and divides a net's
weight among distinct alternatives. Source routed copper is not consulted.

There were nine distinct forecasts for ECC83's nine routable nets and 35 for
PIC's 34 nets. Thus two via preferences offered very little diversity. Coverage
and failed forecasting attempts are explicit; absent forecasts are not proof
that a net needs no space.

Forecast routing took 2.46 s for ECC83 and 27.01 s for PIC, excluding SVG
generation. Complete routing with moderate demand took 44.52 s and 150.63 s,
including native admission and its automatic previews. Some runs overlapped
with other experiments; these are observed wall times, not controlled speed
comparisons.

## ECC83 controls and order experiments

All rows use the same placement, widths, clearances, grid and via geometry.
"Control" uses router-cost tree attachment selection without demand.

| Policy | Complete | Vias | Track mm |
| --- | --- | ---: | ---: |
| Original order and original attachment objective | yes | 10 | 386.308 |
| Original order, control | yes | 8 | 386.851 |
| Original order, demand strength 1 | yes | 3 | 345.004 |
| Original order, demand strength 3 | yes | 5 | 358.277 |
| Pairwise vulnerable first, control | **no: 8/9 nets** | 4 | 320.636 |
| Pairwise vulnerable first, demand 1 | yes | 5 | 347.590 |
| Pairwise vulnerable last, control | yes | 6 | 376.062 |
| Pairwise vulnerable last, demand 1 | yes | 8 | 352.805 |
| Previous-pass cost inflation first, control | yes | 3 | 346.462 |
| Previous-pass cost inflation first, demand 1 | yes | 4 | 347.462 |

The sequential pass commits nets one by one and does not itself backtrack.
Existing order search and coupled repair are separate operations. These order
experiments deliberately restart from identical empty copper.

The 72 directed pairwise probes commit one independently routed net and then
route another. Each probe records added length/via cost, bounded-search
failures, and a copper rendering. On this board every pair was easy: no probe
failed, and the largest individual positive cost increase was only 0.604 mm.
The resulting "vulnerable first" control nevertheless stranded `Net-(P4-P1)`
after eight completed nets. Pairwise ease missed competition among several
nets, and its tiny ranking differences did not predict a good whole-board order.

Learning from an actual full pass was much more useful. Relative to its empty
board forecast, `Net-(P4-PM)` incurred 70.849 mm of added length-plus-via cost
(5 mm per via); `Net-(U1A-K)` incurred 11.243 mm. Promoting expensive nets
produced a complete three-via board without demand costs. Near-zero numerical
differences are rounded away before ranking. Failed and unattempted nets are
explicit categories in the reusable ranking routine.

Demand and order improvements are not additive. The strongest current policy
is a bounded portfolio that retains complete incumbents and learns from whole
passes, rather than selecting a single purportedly hardest-first order.

## PIC holdout

| Policy, original net order | Complete | Vias | Track mm |
| --- | --- | ---: | ---: |
| Original attachment objective | yes | 18 | 1826.911 |
| Router-cost attachment control | yes | 12 | 1833.945 |
| Control plus demand strength 1 | yes | 10 | 1826.835 |

All 34 nets route. Source net-class widths and clearances remain intact. This
forecasting path can route the PIC through-hole connections that the older
completed-copper via importer cannot yet represent; those are separate coverage
boundaries.

## Physical and regression checks

The shared-passage fixture routes A before B. In a 3.0 mm passage, the baseline
centers A and forces two vias on B. Forecast demand moves A aside and both fit
on front copper: 2 → 0 vias for 1.077 mm additional track. The physical minimum
for two 0.8 mm traces with three 0.4 mm spaces is 2.8 mm.

The 2.5 mm negative control retains two vias with or without demand. Four
native PCB-only checks pass with explicit 0.4 mm clearance and 0.8 mm minimum
width rules, zero DRC violations and zero opens. These fixtures have no
schematic, so they do not claim ERC or schematic parity.

All 113 KiCad library tests pass. Demand-specific tests cover own-net
exclusion, leading-slash electrical identity, layer separation, completed-net
removal, branch subdivision/duplication invariance, malformed layer jumps and
cost capping. Thirteen real-board trials were independently read back with
pcbnew: component/pad geometry, net assignments, project/schematic bytes,
trace widths and via dimensions are preserved. Twelve are complete; the one
failed order is excluded from selection. A separate parsed-record audit also
checks that every non-copper board record is unchanged in all thirteen trials.
All 547 SVGs, including the added combined comparisons, parsed. Copper views and
the demand heatmap were visually inspected; browser playback was not tested.

## Reproduction and next implementation step

```sh
python3 experiments/whole-board/run_routing_demand.py SOURCE BOARD_ID SEQUENTIAL_CONFIG OUTPUT
python3 experiments/whole-board/routing_interference_order.py DEMAND_EXPERIMENT ORDER_OUTPUT
python3 experiments/whole-board/demand_channel_control.py CHANNEL_OUTPUT
python3 experiments/whole-board/demand_channel_control.py NARROW_OUTPUT --passage-top 14.5
```

The order driver now includes both pairwise ranking and previous-pass cost
inflation. The latter two runs in this recorded experiment were issued
separately while the first order driver was already running; their exact
configs and terminal results are under `ecc83-observed-orders`.

Keep the new field opt-in while integrating automatic forecasts, learned
orders and complete-incumbent selection into the whole-board coordinator.
Next priorities are actual passage capacity, directional competition, more
diverse forecast embeddings, and updating forecasts after joint congestion
appears. Neither a static heatmap nor a static order should be treated as a
solution to those remaining problems.

# Where the placer A/B noise comes from (E3, E2)

Package `fund-noise`, 2026-11-24/25. This tests E3 and E2 of the
fundamentals note (`build/scratch/fundamentals/fundamental-approaches.md`,
section 5; in the tree as `docs/experiments/fundamental-approaches.md`).

> **Where the code is.** Only this write-up and `experiments/noise-sources/` are on
> main. The counter-based random numbers and the placebo hooks
> (`PCB_PLACER_COUNTER_RANDOM`, `PCB_PLACER_PLACEBO`) stay on branch `fund-noise`
> (d6340d12), because the verdict is not to adopt them. Reproducing E2 needs a
> binary built from that branch; E3 needs only `PCB_ROUTER_SEED`, which main has.

The question: an A/B of a placement change costs 60-100 CPU
hours because the per-board seed noise is large. Can the noise be cut
(E2: random numbers that keep a pair paired), or is the protocol
replicating the wrong thing (E3: placer seeds against router seeds)?

## Verdicts

- **E3: most of a first route's variance comes from the router, not
  from the placement.** On the three boards the placer seed's own
  effect is 0 %, 22 % and 30 % of the variance of one first route,
  about 17 % on average. The router seed's main effect is about 11 %,
  and the placer x router interaction about 72 %. Two router seeds agree
  on the order of four placements at a mean Spearman of +0.16. One router
  seed picks the placer seed that is best on the mean in 6 of 12 cases,
  which is what chance gives with four seeds. The router seed also
  changes which placement the race keeps: one placer seed under four
  router seeds routed 2 or 3 distinct placements in 11 of 12 cases.
- **E2: pairing by seed removes no variance today, and the counter-based
  random numbers do not change that.**
  - A change that should not matter moves almost every part: a 0.1 mm
    start nudge, or one extra discarded draw. The median is 98-100 % of
    the movable parts, with the sequential and the counter-based anneal
    alike.
  - The first-open change of such a placebo pair is as large as the
    difference between two independent placer seeds. On OpenESC mini it
    is 14-16 against 14; on glasgow 12-16 against 9.
  - The counter-based random numbers remove the stream desync exactly:
    the extra-draw placebo becomes a no-op on all 10 pairs. But a
    physical change still decorrelates: the anneal, the legaliser and
    the race are chaotic of their own accord. Only 1 of 10 counter-based
    nudge pairs stayed identical.
- **Recommendations.**
  - (a) **Do not replace the sequential random numbers with the counter
    ones.** They buy nothing measurable. In 3 of 8 protoesp placements
    the counter mode left a part illegal (`REF**`, or `SW1` and `SW3`),
    against 0 of 12 sequential ones. This is unexplained and would need
    to be explained before any use. Keep the prototype as an off lab
    switch, or delete it.
  - (b) **`placer_ab.py`'s model stands.** Treating the pairs as
    independent draws is correct, and its pair counts stay. Routing one
    placement with two router seeds saves at most 2-20 % of the CPU at
    equal resolution. That is not worth tooling until the placer's share
    is known better.
  - (b) **For the race, test router-seed replication; this is not yet
    established.** The race already picks among realised routings, and
    the kept probe routes on. More router seeds per placement are more
    tickets of the same lottery, as `PCB_ROUTER_SEEDS` is for routing.
    Whether two placements x two router seeds beat four placements x one
    seed at the same probe budget is a stage-1 question. Do not argue it
    from these numbers.

## Setup

- **Boards:** three dense boards from `dense-noise.json`, chosen as the
  cheap ones with real noise: `CRImier__MyKiCad__protoesp` (protoesp),
  `OpenDrone-hw__OpenESC-20x20__4in1-mini` (mini) and
  `GlasgowEmbedded__glasgow__glasgow__revC3` (glasgow).
- **Cells:** each cell is one layout stopped after the first route, the
  placer ladder's stage 1. It runs through run.py: tasks v7, pinned
  rates, `--threads 2`, the memory ledger. The metric is
  `first_unconnected_terminals`.
- **Scripts:** `experiments/noise-sources/grid.py` runs the plans and
  `analyze.py` makes every table here.
- **Binaries:** main's nightly binary (96e891b2) for the identity check.
  This branch's binary with the switches off for E3 (code of 04e280f1).
  This branch rebuilt at 437b6943 for E2.
- **Placer seeds:** 1, 101, 201, 301. **Router seeds:** 1001-1004
  (`PCB_ROUTER_SEED`).

What `PCB_ROUTER_SEED` varies (`pcb-router`, `router/order.rs` and
`router/build.rs`):
- the routing order: each net's span times U(0.6, 1.4);
- the history maps, filled with noise of one history increment.

It applies to every router whose `config.seed` is unset. That is the
race's probes and the first route, and seed index 0 where several seeds
route side by side. Unset, the order is the plain span order and the
history starts at zero.

What the placer seed varies: `tuning.seed` sets the global placement's
seed, `N + k` for race seed k. The anneal's seed is `7 + k` whatever N
is (`board_placer/place/seeds.rs`). So a placer seed changes the global
start's noise, and through it everything after it, including every
router run.

## Checks

- **Identity.** Main's binary and this branch with the switches off give
  the same output on protoesp seed 1: placement digest `c3d1f360…`,
  first open 41 and 41, 0 of 228 footprints moved.
- **Determinism.** `e3-protoesp-p101-r1002` run a second time gives the
  same digest, the same first open (7), the same probes, and a routed
  `.kicad_pcb` that is byte-identical once the time stamps are ignored.
  The interaction term of E3 is therefore real chaos between placement
  and router, not run-to-run noise.

## E3: placer seed x router seed

First-route open terminals. Rows are placer seeds, columns router seeds.

| protoesp | 1001 | 1002 | 1003 | 1004 | mean |
|---|---|---|---|---|---|
| 1 | 11 | 13 | 8 | 7 | 9.8 |
| 101 | 28 | 7 | 18 | 13 | 16.5 |
| 201 | 15 | 13 | 13 | 13 | 13.5 |
| 301 | 26 | 35 | 9 | 19 | 22.2 |

| mini | 1001 | 1002 | 1003 | 1004 | mean |
|---|---|---|---|---|---|
| 1 | 168 | 169 | 144 | 177 | 164.5 |
| 101 | 156 | 153 | 121 | 150 | 145.0 |
| 201 | 154 | 137 | 125 | 160 | 144.0 |
| 301 | 150 | 141 | 144 | 132 | 141.8 |

| glasgow | 1001 | 1002 | 1003 | 1004 | mean |
|---|---|---|---|---|---|
| 1 | 86 | 87 | 131 | 93 | 99.2 |
| 101 | 93 | 96 | 94 | 114 | 99.2 |
| 201 | 89 | 106 | 110 | 96 | 100.2 |
| 301 | 96 | 91 | 85 | 110 | 95.5 |

The variance decomposition uses a two-way random-effects model without
replication. A cell is deterministic, so the interaction is the residual.
Components are floored at 0.

| board | placer | router | interaction | sd of one cell | mean |
|---|---|---|---|---|---|
| protoesp | 15.1 (22 %) | 0.8 (1 %) | 51.3 (76 %) | 8.2 | 15.5 |
| mini | 84.1 (30 %) | 85.6 (31 %) | 108.6 (39 %) | 16.7 | 148.8 |
| glasgow | 0 (0 %) | 0 (0 %) | 194.2 (100 %) | 13.9 | 98.6 |

The same data as root-mean-square differences between two cells:

| board | same placer seed, other router seed | same router seed, other placer seed |
|---|---|---|
| protoesp | 10.2 | 11.5 |
| mini | 19.7 | 19.6 |
| glasgow | 19.5 | 17.3 |

Changing only the router seed moves a first route as far as changing the
placement does.

Limits: each board has df 3 / 3 / 9, so a component's estimate is rough.
glasgow's placer and router mean squares (17.6 and 177.7) lie at or below
the residual's (194.2). The placer share is somewhere between 0 and
about 30 % on these boards; it is not a precise 17 %. The router's share
includes the race's pick, which the router seed changes.

### The protoesp "unset" anomaly

The identity cell at placer seed 1 with `PCB_ROUTER_SEED` unset left 41
open, against 7-13 with any router seed set. The stored runs explain it
without new runs.

- At placer seed 1 the race probes two placements: 0, the placer's
  first-ranked, and 1, an alternative.
- With the router unset, probe 0 left 73 open and 87 after the rip-up;
  probe 1 left 19 open and 158 after the rip-up. The race ranks by the
  ripped count, so it kept placement 0, which routed on to 41.
- With router seed 1002 the race also kept placement 0. Its poses are
  the same as the unset run's (0 parts differ). It routed to 13. Same
  placement, other router seed: 41 against 13.
- With router seeds 1001, 1003 and 1004 the ripped count favoured
  placement 1 (40 against 157, 23 against 28, 52 against 154), which
  routed to 11, 8 and 7.

"Unset" does nothing special beyond the mechanics above: no order jitter
and a zero history. It is not systematically worse.
- protoesp at seeds 101, 201 and 301 with the router unset gave 8, 7 and
  12, against seeded means of 16.5, 13.5 and 22.2.
- mini gave 135, 122, 147 and 134 unset (mean 134.5), against a seeded
  mean of 148.8.
- glasgow gave 94, 98, 107 and 92 (mean 97.8), against 98.6.

The 41 is one unlucky (placement, router) draw that the race kept. It is
also a live example of the race's ripped count ranking two placements in
an order that one router seed reverses.

## E2: does pairing by seed remove variance?

### The prototype

`AnnealConfig::counter_random` is a lab switch, set with
`PCB_PLACER_COUNTER_RANDOM=1` and off by default.
- Each draw is splitmix64 of (a key from the seed, the stage, the move
  within the stage, the draw's index within the move).
  `Random::begin(stage, step)` starts each move.
- The bus groups' attempts are moves numbered from 2^40 on.
- The sequential mode is the old xorshift; a unit test checks this bit
  for bit.

The other placer random numbers needed no change. The global start
already draws a fixed number per body, and draws for a warm part all the
same.

The placebos are set with `PCB_PLACER_PLACEBO` (`AnnealConfig::placebo`):
- `draw`: one extra discarded draw at the start of the anneal's stage
  `stages / 4`. It is a no-op by construction in counter mode.
- `nudge`: the first movable two-pin part starts every anneal 0.1 mm
  further right. This is a physical change that should matter almost
  nothing.

Each pair compares a placebo cell with its base at the same board,
placer seed and random-number mode. The router seed is unset on both
sides, as in production.

### Results

The share of movable parts moved is measured on the routed placement's
poses; the probes changed are the race probes whose result differs.

| rng | placebo | pairs | identical routed placements | median share of movable parts moved | probes changed | sd of the first-open change |
|---|---|---|---|---|---|---|
| sequential | draw | 12 | 3 | 98 % | 21 of 32 | 12.6 |
| sequential | nudge | 12 | 0 | 100 % | 28 of 30 | 13.7 |
| counter | draw | 10 | 10 | 0 % | 0 of 26 | 0.0 |
| counter | nudge | 10 | 1 | 98 % | 25 of 26 | 12.5 |

Root-mean-square first-open change per board, against two independent
placer seeds under the same settings (the bases of different seeds). The
number of pairs is in brackets.

| board | seq: independent | seq: draw | seq: nudge | ctr: independent | ctr: draw | ctr: nudge |
|---|---|---|---|---|---|---|
| protoesp | 22.8 (6) | 2.5 (4) | 15.2 (4) | 1.0 (1) | 0.0 (2) | 0.7 (2) |
| mini | 14.4 (6) | 14.5 (4) | 14.1 (4) | 14.3 (6) | 0.0 (4) | 15.8 (4) |
| glasgow | 9.4 (6) | 16.1 (4) | 11.7 (4) | 9.0 (6) | 0.0 (4) | 11.8 (4) |

protoesp's columns are dominated by the 41 above and by the counter
cells that failed (below). On mini and glasgow every placebo pair
differs as much as, or more than, two independent seeds. The per-pair
table is in `analyze.py e2`.

Further observations:
- **The sequential `draw` desync is real.** In 9 of 12 pairs one extra
  draw moved the routed placement almost entirely. In the other 3
  (protoesp 1 and 301, mini 1) the race routed the same placement; the
  draw changed other candidates only, or the kept placement's anneal
  ended before the stage of the draw.
- **The counter mode stays local only in exceptional cases.** In one
  case (protoesp 201) the nudged part was seated back and the whole
  placement stayed identical. Everywhere else, a 0.1 mm difference grew
  through the anneal's accept and reject decisions, the legaliser's
  order and the race's pick. The fundamentals note's hope that "at the
  anneal's low temperature the difference tends to stay local" does not
  hold.
- **The counter mode left parts illegal on protoesp.**
  - Seeds 1 and 101, base and `draw`: illegal `REF**`. These are the
    same placement per seed, since `draw` is a no-op there.
  - Seed 1, `nudge`: unplaced `SW1` and `SW3`.
  - That is 3 of 8 distinct counter-mode placements against 0 of 12
    sequential ones.
  - All three race seeds of such a layout must have failed for the
    layout to fail. Whether this is chance or a defect of the scheme (for
    example a correlation between the draws of one move that the
    sequential stream does not have) is not known.
- On the boards where both modes placed legally, the base first opens
  are alike. mini: sequential 135, 122, 147, 134; counter 133, 144, 137,
  156. glasgow: sequential 94, 98, 107, 92; counter 81, 89, 76, 89.
  glasgow's counter mean is 14 lower, which is within its noise at four
  seeds.

## What it means for the protocol

**Pairs.** With either random-number mode, the two sides of a pair are
independent draws. A pair's difference has a variance of about twice a
cell's variance, and `placer_ab.py`'s noise model and pair counts are
right as they are.

Per board, the relative sd of one first route is 53 % (protoesp), 11 %
(mini) and 14 % (glasgow). Resolving a 5 % effect on one board at a
one-sided 3 % level with 80 % power would need ((1.88 + 0.84) x sqrt(2)
x cv / 0.05)^2 pairs: about 72 on mini and 116 on glasgow. Pooled over
the eleven boards, that is the 5- and 9-pair rule's known power (a 5 %
effect is found about 55 % of the time). The coupled random numbers would
have cut these counts only if the pair variance had fallen; it did not.

**Router seeds in `placer_ab.py`.** A placer-seed pair already re-rolls
the router on both sides, so it samples router noise anyway. Routing one
placement with k router seeds lowers a pair's variance from
2(P + R + I) to 2(P + (R + I)/k), where P, R and I are the placer,
router and interaction components. It costs 2(f + k(1 - f)) cells, where
f is the placement's share of a cell's cost: 40-65 % of the wall time
here (placement 137-233 s of 303-379 s).
- With P at 17 % and f at 0.5, two router seeds cost 1.5 times as much
  and give 0.59 times the variance, about 12 % less CPU at equal
  resolution.
- With P at 5 % the saving is 21 %; with P at 30 % it is 2.5 %. Three or
  more router seeds save less.
- It would need a way to route one stored placement under several router
  seeds in layout mode. `--reuse` keys on `PCB_ROUTER_SEED`, so it does
  not help.

**The race.** Even when it ranks by the ripped count, the race compares
placements by one realised routing each. E3 shows that one realisation
says little about a placement's mean: an agreement of +0.16, and the
same placement went from 41 to 13 under another router seed. But the
race does not need the best mean. Its kept probe routes on, so picking
the best realised routing is a legitimate best-of-n. Replicating router
seeds in the race (n placements x m seeds, keeping the best realised
probe) adds tickets of the same kind that more placements add. Which mix
is better at a fixed probe budget is the experiment to run next, on the
stage-1 protocol. It is cheap to try, because the race already routes
probes side by side.

## Reproducing

```sh
export TMPDIR=/home/flo/programming/pcb-maker/build/scratch/tmp-fund-noise
experiments/noise-sources/grid.py run identity --parallel 2
experiments/noise-sources/grid.py run e3            # 48 cells
experiments/noise-sources/grid.py run e3-repeat
experiments/noise-sources/grid.py run e2 --binary <a build of this branch>   # 72 cells
experiments/noise-sources/analyze.py identity|e3|e2
```

The cells and their logs are in
`/home/flo/programming/pcb-maker/build/scratch/fund-noise/runs/`. The
total was 123 cells (5 of them ended at the placement), at 410-485 CPU seconds a first route on average
(about 15 CPU hours).

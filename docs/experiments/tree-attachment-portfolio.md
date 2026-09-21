# Bounded shared-tree attachment portfolios

The predecessor shared-tree router greedily searched from the single
tree-vertex/terminal pair with the lowest grid-distance estimate. That is a
good default, but it charges motion along already-owned tree copper during A*
even though the prefix is removed before commit. A nearby source can therefore
win the estimate while producing a longer retained branch.

The opt-in Rust portfolio keeps the predecessor ordering as its candidate
producer, then:

1. retains geometrically distinct sources using an explicit spacing radius;
2. evaluates at most `maximum_tree_attachment_searches` sources;
3. trims every successful route through its last same-net contact;
4. compares the retained branches with a selectable objective;
5. commits only the selected branch through the existing exact split/junction
   transaction.

The selectable objectives are `retained_length`, `via_count_then_length`, and
`router_cost`. The bound defaults to one, so existing shared-tree behavior and
work remain unchanged unless the experiment is enabled. Evidence retains every
source, target, status, search/expansion count, retained length, via/bend count,
trim result, and selection decision. The viewer draws selected sources in blue
and rejected sources in gray.

## Reduced positive control

`shared-tree-attachment-portfolio.json` first builds a vertical A–B trunk. C
is across an asymmetric wall. Manhattan distance selects `(5,10)`, but a
second source two millimetres away at `(5,8)` produces a shorter route around
the lower end of the wall.

Measured with the checked-in configurations on 2026-09-01:

| Policy | Attachment searches | Expansions | Copper | Exact result |
| --- | ---: | ---: | ---: | --- |
| Greedy predecessor-compatible source | 1 | 368 | 19.242641 mm | pass |
| Two-source, 2 mm spacing, retained-length objective | 2 | 500 | 17.828427 mm | pass |

The portfolio saves 1.414214 mm (7.35%) for 132 additional expansions
(35.9%). The selected alternative is an interior tree point, so the result is
not merely the already-ported terminal-junction preference.

## Controls and interpretation

The same two-source configuration does not help every tree:

| Fixture | Greedy expansions / copper | Portfolio expansions / copper |
| --- | --- | --- |
| Dynamic-terminal control | 135 / 66.000000 mm | 279 / 66.000000 mm |
| Prefix-trim control | 1,303 / 44.863961 mm | 1,957 / 44.863961 mm |
| Terminal-junction control | 26 / 12.414214 mm | 42 / 12.242641 mm |

The first two are explicit work penalties with no quality gain. This is why the
portfolio is bounded and disabled by default. The terminal control reaches the
same result as the semantic terminal-only experiment at the same total work;
the general portfolio remains useful because the positive control selects a
non-terminal source.

Run the positive control with:

```sh
cargo run --release -- route-grid \
  benchmarks/small/shared-tree-attachment-portfolio.json \
  run/shared-tree-attachment-portfolio.json \
  experiments/configs/dut-grid-shared-portfolio.json
```

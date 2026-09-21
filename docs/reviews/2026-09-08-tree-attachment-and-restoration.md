# Tree attachment and the larger-board restoration conflict

The next whole-board completion failure occurs when inserting
`Net-(C201-Pad2)` requires yielding `+12V`, but `+12V` cannot then be restored.
The retained incumbent has 27 routed nets and 36 native open items. This study
keeps its placement, source net classes, widths, clearances and all unrelated
copper fixed.

## Attachment controls

At 0.5 mm resolution every attachment policy fails on branch zero. At 0.25 mm,
the existing eight-source shared-tree policy reaches branch three. Rooted-star
routing fails on branch one; 32 sources spaced by 4 mm and 64 sources spaced by
1 mm both still fail on branch three. These are routing-grid failures, not
proofs of physical infeasibility. The finer controls preserve physical bend/via
costs by scaling the corresponding grid costs.

The production adapter now has an opt-in `tree_attachment_search: "multi_source"`
mode. It seeds one A* request with every represented materialized tree contact,
including the opposite layer where a plated pad proves contact. This removes
sampling limits within the represented tree without inventing copper or drilled
vias. It requires `shared_copper_tree`, `router_cost`,
`maximum_tree_attachment_searches: 1` and
`tree_attachment_minimum_spacing_mm: 0`. The ranked-sample default remains
unchanged, including its serialized configuration. Two-terminal portfolio
normalization recognizes that attachment policies are equivalent for those nets.

The multi-source restoration still fails on branch three, with 936,994 A*
expansions and 323,694 preflight expansions. The eight-source control uses
1,052,351 and 524,990 respectively; expanding to 64 sources uses 3,200,520 and
7,458,668. These aggregate counts include earlier successful branches. Changing
attachment sampling alone does not resolve the conflict in these contexts.
The multi-source search has a different tree-building trajectory; this is not
an exhaustive comparison of all possible trees, grid phases or placements.

## Complete-board control

Cold ECC83 routing completes all nine nets with both policies. The multi-source
run uses 199,061 A* expansions versus 2,227,164 for the default policy, a 91.1%
reduction. The final native board is byte-identical: 439.744409 mm of track and
11 vias at unchanged placement. This is a search-work result on one complete
board, not an end-to-end wall-time or general performance claim. The default
also reproduces the previous frozen ECC83 control. Native readback checks pose,
unrelated copper, dimensions, rules and connectivity. The KiCad library suite
passes 127 tests, including shared-junction materialization under both policies.

## Broader repair experiment

With attachment-only controls exhausted, the next experiment tries yielding
one additional net while restoring `+12V`, followed by restoration of that net.
The diagnostic exhausts all 27 eligible retained nets. Eight removals permit
`+12V` routing; that alone is not progress, since each removed net must be
restored. Each materialized trial gets native verification and a combined
copper rendering. Only `Net-(U102-CAP+)` can be restored after `+12V` is inserted.
That complete transaction advances the original board to **28 routed nets /
34 open items**. The inner audit checks every alternative, dimensions and native
selection; the outer audit compares the selected result with the original
27-net board. It preserves component poses, project/schematic/rule inputs and
every copper item outside `Net-(C201-Pad2)`, `+12V` and `Net-(U102-CAP+)`.
Seven existing annotation findings remain, with no new findings or
disconnections. Full-board completion is still false.

This uses the existing single-net repair recursively as an explicit experiment.
At this stage the automatic coordinator did not invoke nested restoration or
understand its additional changed-net receipts. The subsequent
[automatic integration](2026-09-08-nested-routing-recovery.md) adds bounded
nesting, preserves restoration obligations and carries the full changed-net set
through admission, reporting and resume audits. It reproduces this 28-net board
and continues to 34 nets / 24 opens. This experiment
combines finer resolution, multi-source routing and an additional yielding net;
their individual contributions to this successful transaction are not isolated.

[Run report and combined views](../../build/multi-source-tree-2026-09-08/index.html)
retain configurations, executable/source hashes, all failed controls and native
artifacts. This experiment targets whole-board completion; local via polishing
remains deferred.

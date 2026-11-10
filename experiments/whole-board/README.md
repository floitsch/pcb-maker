# Freerouting DSN helpers

What is left of the whole-board reference suite of September 2026 (its
runners called research commands removed on 2026-10-20 and 2026-11-10;
the rest is in git history before 2026-11-10). These files stay because
the external-router harnesses use them:

- `benchmarks/github/freerouting.py` compiles the project's net classes
  (`compile_net_classes.py`, KiCad's `pcbnew` module) and translates them
  into Freerouting's DSN (`translate_dsn_classes.py`, which reads the DSN
  with `audit_dsn_classes.py`).
- `benchmarks/tscircuit/run_board.py` imports a Freerouting session into
  the board with `compare_native_router.py --native import`.

`compare_native_router.py` keeps its own comparison mode with its helpers
(`translate_dsn_edge.py`, `inspect_dsn_geometry.py`,
`render_inspection_layers.py`, the Java companions); `run.py` holds the
small file helpers they share (`command`, `digest`, `read`, `write`) and
the old suite runner over `benchmarks/real/whole-board.json`. The tests
(`test_*.py`) cover the class compiler, the edge translation and the
router deadline.

Autorouter comparisons are finished (2026-10-23); docs/benchmarks.md has
the head-to-heads that stand.

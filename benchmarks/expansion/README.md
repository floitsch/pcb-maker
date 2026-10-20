# Board collection and automatic evaluation

This is a separate intake collection, added 2026-10-18. Existing benchmark
and training manifests are unchanged. Downloaded designs, logs, binaries,
features and results live in **`data/`**, which is ignored by git and stored
on disk. Temporary files also stay there: this host's `/tmp` is RAM-backed.
No downloaded repository scripts or build systems are executed.

## Start, inspect and stop

```sh
python3 benchmarks/expansion/run.py --hours 24 --workers 28
python3 benchmarks/expansion/status.py
touch benchmarks/expansion/data/STOP
```

The supervisor starts downloads plus a persistent eight-worker quick pool.
The other 20 worker slots perform follow-up work. These are maximums:
shared memory admission, a 10 GiB reserve, disk reserve and load checks can
reduce concurrency. Routing children use one search thread and nice 19;
native KiCad checks can use helper threads and have separate concurrency
limits. The existing benchmark is left running. A stop request terminates
owned child process groups; completed jobs survive a restart. Remove
`data/STOP` deliberately before restarting. A supervisor also stops after
24 hours by default, including its downloader.

The launch made during this task is detached from the terminal. Its PID,
deadline, pinned binary SHA-256 and repository revision are recorded in
`data/supervisor.json`. `supervisor.log`, `harvest.log` and `quick.log`
show progress. Per-stage status files update while a batch is running.

## What happens automatically

1. **Acquire.** Revisit the existing candidate backlog, excluding boards
   already accepted in the old GitHub benchmark/training sets. Download
   native boards and their project/custom-rule sidecars at pinned commits.
   Discover more repositories through topics and scan complete repository
   trees, including repositories already partly represented. Hash duplicates
   are recorded. Every outcome is journaled and accepted boards are published
   atomically in `data/manifest.json`.
2. **Quick route.** Preserve the designer's placement. Remove tracks, arcs,
   vias, cached fills and small hand-drawn copper zones using pcb-maker's
   existing strip command; retain large pour declarations and keepouts.
   Route with a **30-second work budget**. The internal wall guard is 90 s
   and the whole-process guard is 180 s; loading, checks and scheduling
   delays are recorded separately. This avoids calling a slow import or a
   busy-machine wall cut a difficult routing problem.
3. **Check.** Keep source and routed native KiCad DRC reports separately.
   A clean routing label requires native connectivity and no non-cosmetic
   error. Source findings, inferred rules, importer failures, internal
   violations, memory limits and timeouts are distinct fields/categories.
   This intake does not perform schematic ERC or certify electrical design.
4. **Follow up.** Reattempt memory-limited boards in four-worker batches
   with a higher memory cap. Route selected nontrivial cases with a
   **300-second work budget**, recording changes in completion, vias and
   length. Batches advance rather than repeating the first selected boards.
5. **Placement data.** For eligible boards, save features and exact boards
   for the designer placement plus three deterministic legal perturbations.
   Route those saved variants for up to 300 work seconds. Their input
   hashes join features to outcomes. Exported features alone never imply a
   success label; incomplete, invalid and missing-check outcomes remain
   explicit. Functional/mechanical placement constraints beyond those in
   the source are not inferred by these perturbations.
6. **Other formats.** Retain Eagle/Altium originals; try native conversion
   in bounded batches. Read-back failures are quarantined. Successful
   conversions are experimental routing inputs with inferred rules and
   disabled automatic neural-training eligibility until semantic fidelity
   is established. See [FORMATS.md](FORMATS.md) for real tests and the
   Konnect assessment.

## Selection axes

| Outcome or property | Intended use |
|---|---|
| Native-clean routing in at most 3 routing seconds | Simple/easy training example; low priority for router benchmarks |
| Native-clean routing in 3–30 seconds | Moderate regression/training example |
| Still incomplete at 30 work seconds | Benchmark candidate when source checks and original rules are good |
| Finishes after more work | Useful difficulty curve and routing-quality evidence |
| Native source findings or missing original rules | Review queue; difficulty remains provisional |
| Import/model/outline/conversion failure | Importer regression; no false router-difficulty claim |
| Memory limit or wall timeout | Resource/performance queue; not a verified routing failure |
| Layer/package/pad counts, fixture/panel/backup flags, source family | Diversity and sampling information |
| Repository licence metadata | Conservative collection flag; per-file exceptions remain unchecked |

Fast boards are retained. Local escape and placement examples can be useful
even when the full board is easy. Selection should consider diversity and
source quality alongside routing time. Existing comparison suites do not
automatically acquire these boards: `benchmark_candidate` is an admission
proposal, not promotion into a published benchmark.

Placement data uses a deterministic **80/10/10 repository-family split**.
All revisions from a repository stay together. This is conservative for
large collections; shared designs across different repositories still need
family auditing. Source files are not redistributed by this repository.

## Artifacts and reproducibility

- `data/harvest.jsonl`: source/rejection/duplicate/conversion-pending journal.
- `data/manifest.json`: accepted native intake boards and pinned provenance.
- `data/runs/{quick,high-memory,deep,placement-labels}/`: results, status,
  input snapshots, configs, logs, native reports and pinned binary hashes.
- `data/samples/`: immutable placement feature arrays, exact variant boards,
  family splits and the variant routing manifest.
- `data/conversions/`, `data/converted-manifest.json`: experimental imports.

Every triage fingerprint includes actual board and rule-file contents,
binary/configuration and runner/native-CLI hashes. Changes create new
results. Original source files are preserved. Follow-up sampling is bounded
to 128 new boards per cycle by default; downloads and quick triage continue
while those jobs run.

```sh
python3 -m unittest discover -s benchmarks/expansion -p 'test_*.py'
```

The tests cover process cleanup, native-label gates, actual-content resume,
deduplication/retry, discovery completeness, conversion quarantine, and
advancing/accumulating data batches. Real smoke runs additionally exercised
downloads, successful native routing, resource limits, feature export and
routing of exported placements.

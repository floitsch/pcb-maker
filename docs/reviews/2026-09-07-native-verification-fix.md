# Native verification repair

The report-execution and report-validation defects identified in the
[program review](2026-09-07-program-review.md) are fixed in the Rust program.

Native report handling now lives in
[`native_report.rs`](../../crates/pcb-kicad/src/native_report.rs). Each command
writes to a new temporary directory. Only successful execution with a fresh,
valid report can publish `erc.json` or `drc.json`. The commands omit
`--exit-code-violations`: execution status establishes whether the check ran;
the findings establish whether the board passes. A successful command with no
report is an error, and output from a failed command is not accepted.

Validation checks the required report structure and source filename before
interpretation, including the nested ERC sheets and all three DRC finding
arrays. It checks the fields admission/diagnosis depend on; this is not a full
JSON Schema implementation. Unknown metadata is preserved. When KiCad provides
included-severity metadata, it must contain all requested severity classes.
Reports from older KiCad versions may omit that optional metadata. Structure
is based on the official [ERC](https://schemas.kicad.org/erc.v1.json) and
[DRC](https://schemas.kicad.org/drc.v1.json) schemas.

Re-verification invalidates the previous `verification.json` first. The same
validation applies to frozen ERC reports and cached native reports. A cache hit
recomputes the assessment and compares it with the stored summary; matching raw
report hashes alone are insufficient. The verifier implementation digest now
includes the new module, invalidating caches produced by the earlier code.

## Evidence

- All 84 `pcb-kicad` library tests pass, including five new native-report tests
  covering failed/missing/malformed output, required structure, wrong report
  identity, filtered severities, finding preservation, and stale verdict removal.
- Both new CLI integration tests pass: a previously complete board cannot retain
  its verdict after execution failure, and a cache summary that disagrees with
  its native reports is rejected.
- Repeating the two original fault injections with the repaired executable
  returns exit 1 and leaves no `verification.json` in either case. Retained
  [results](../../build/native-verification-fix-2026-09-07/fault-injection.json)
  can be compared with the
  [original failures](../../build/program-review/2026-09-07/results.json).
- The native shared-tree insertion integration test passes with KiCad 10.0.6.
- The native dual-ESP32 cold-prefix integration test fails because its source
  board has two `silk_edge_clearance` warnings at U1/U2. Running the unchanged
  release executable on a separate copy rejects the same input with the same
  two findings. The [control](../../build/native-verification-fix-2026-09-07/preexisting-dual-control.json)
  records that pre-existing fixture failure; the acceptance policy was not
  relaxed to hide it.

- The longer native transaction test verifies commit, rejected-candidate
  recovery, exact rollback, and 13 successfully admitted connections, then fails
  its historical assertion that the final connection must take two attempts.
  The current router commits that connection locally on the first attempt.
  The unchanged release executable independently finds a different local route
  on the same input, and that route also passes native admission. This
  [control](../../build/native-verification-fix-2026-09-07/preexisting-rung13-control.json)
  supports updating the strategy-specific fixture assertion separately; it is
  not a claim that the two executables produce identical routes. The original
  test assertions were not changed by this repair.

Consequently the three existing native tests report one pass and two failures,
with the two diagnoses above. Reproduction commands:

```
cargo test -p pcb-kicad --lib
cargo test --test native_verification
cargo test --test kicad_ladder native_
```

## Remaining integration work

This fixes report acceptance, not the whole architecture review. Atomic
publication of a complete board revision and its reports is still outstanding.
The next representation work must preserve existing cycles and source-object
coverage, expose rule-aware queries, and support explicit local patches. The
via-only keepout omission was subsequently
[repaired](../../benchmarks/competitive/mechanisms/via-only-keepout.md). The
dual-board silkscreen source failure was subsequently
[repaired in the template generator](2026-09-07-reference-edge-fix.md), allowing
the two-connection cold prefix to complete. The longer transaction test's
strategy-specific assertion remains a separate issue.

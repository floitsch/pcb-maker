# Verified sequential checkpoint resumption

Interrupted searches can now continue from their last committed board using
`resume-kicad-board-sequential <checkpoint-directory> <output-directory>
<resume-config.json>`. This avoids routing the successful prefix again while
preserving the native admission requirements.

The wrapper configuration contains `sequential` (the normal sequential router
configuration) and `allow_existing_annotation_findings` (default false).
`maximum_connections` includes inherited commits; the repair invocation budget
applies to the new invocation. An empty order inherits the previous order.
Only the remaining order can change. Output must be a fresh directory outside
the checkpoint and original source.

The implementation checks a contiguous committed prefix, board hashes, pad/net
inventory, project/ERC inputs, and unchanged fixed geometry. It selects the
last committed admission snapshot, ignoring any unfinished repair alternatives
or torn mutable result. It performs fresh native verification and creates the
usual combined-copper preview before continuing. New checkpoints are written
through a temporary file and atomic rename. Historical artifact directories
remain dependencies: retain the original archive unchanged. Resuming a running
writer concurrently is not covered by these controls.

The bounded larger-board continuation retains 25 committed nets and adds VCC,
reducing native open items from 39 to 37. Seven existing annotation findings
remain; this is a 26/50-net partial result, not a completed board or a finished
adaptive policy comparison. The continuation deliberately allows only one new
net and one repair invocation with four diagnosis trials. Production limits
are unchanged. The earlier demand-guided run remains ahead at 27 nets / 36
opens.

Native controls reject reordered committed nets and a changed committed board
hash. A deliberately corrupted mutable output is ignored in favor of the
verified commit. Validation-only resumption retains 25 nets / 39 opens. Completed
ECC83 resumption and the cold ECC83 control both finish all nine nets with no
native design/connectivity findings. Partial commands return exit status 1 even
when the requested bounded continuation succeeds; inspect `termination` and
native receipts rather than interpreting that status as a rejected checkpoint.

The Rust library suite passes 126 tests. Frozen executable and source hashes,
control exit records, native reports and automatically generated renders are in
[the run directory](../../build/checkpoint-resume-2026-09-08/).
[Combined views and audit results](../../build/checkpoint-resume-2026-09-08/index.html)
record the independent readback checks.

The standalone continuation's original process handle disappeared before its
exit status was collected. Its final report and fresh native artifacts exist,
and no router/KiCad CLI process remained on inspection; no numeric process exit
is inferred from those files. The supervised controls retain actual child exit
statuses in `controls.json`.

This closes the interrupted-run infrastructure work. The next capability target
is complete routing of the larger board, using the remaining restoration
conflict to drive broader repair or placement choices. Via polishing remains
a deferred optimization.

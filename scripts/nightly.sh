#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# The nightly measurement of main (benchmarks/agent-tasks/README.md,
# "Nightly"): builds main at its head, snapshots the binary and runs the
# tiers through the memory ledger (no lock), then writes a summary and
# publishes it into the docs on main (step 5).
#
#   bash scripts/nightly.sh                  # tonight's measurement
#   bash scripts/nightly.sh --quick          # without the sweep and the designer tier
#   bash scripts/nightly.sh --install        # installs the systemd user timer (02:00)
#   bash scripts/nightly.sh --print-units    # prints the units --install would write
#   bash scripts/nightly.sh --no-commit-report   # measure and summarise, publish nothing
#
# What it does:
#   1. Refuses when the main checkout has uncommitted changes to tracked
#      files. Builds main's head in a checkout of its own,
#      ../pcb-maker-nightly (a detached worktree, created on the first
#      night with the main checkout's build, artifacts, harvested boards
#      and expansion data linked in, as scripts/worktree.sh does; it is
#      the nightly's alone: each night checks it out at main's head with
#      --force), so neither main's target/ nor an agent's worktree is
#      touched. Copies the binary to
#      build/bin/pcb-maker-nightly-<date>-<sha12> and keeps the newest 7
#      of those. When the newest nightly measured the same commit with
#      every stage planned tonight, it stops there (--force runs anyway).
#   2. Runs, one after the other, each through the memory ledger (no
#      lock) with run.py's measured floor of available memory (since
#      2026-10-27; 10 GB until then, and without headroom samples) and
#      --ledger-priority nightly, from the nightly checkout. The priority puts the nightly's boards first
#      among the ledger's waiters and has the ledger drain for them at
#      once: a newer request (another agent's route, a sweep's board) goes
#      only while it leaves the waiting board its room, so the night ends
#      within the night (docs/benchmarks.md, "Memory ledger"; until
#      2026-10-25 the designer tier's large boards waited from 11:00 to
#      past 20:00 behind other agents' 1-8 GB routes):
#        fingerprints  Castor, k30-SBC, jetson, Sisu in route mode from
#                      build/github-layout-v16, 1 x 4, budget scale 0.25,
#                      PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1
#        expansion     --tier expansion, 8 x 1 (the designer's placement)
#        short         --tier short, 3 x 2
#        designer      --tier designer, 8 x 1, --quality, --order
#                      peak-desc, --budget-scale the median
#                      ladder_budget_share of the newest earlier night's
#                      sweep (0.695 without one)
#        sweep         tasks/v7, 10 x 2, --quality, --order peak-desc (the
#                      full layout sweep, its largest boards first)
#      Every stage declares each board's expected peak from the nightly's
#      own estimates, build/nightly/estimates.json (step 4), else from the
#      committed benchmarks/agent-tasks/estimates.json (run.py --estimates;
#      README.md, "How to measure").
#      The designer tier runs before the sweep and its largest boards
#      first (since 2026-10-26): it is the stage with the large boards
#      (up to 6.5 GB measured) and the shorter one (about 46 000 s of
#      boards against the sweep's 91 000), so it runs while the machine is
#      quiet; after the sweep it began at 06:27-10:29 and its large boards
#      waited into the agents' day (on 2026-10-25 past midnight).
#      The sweep and the designer tier run on the days of --sweep-days
#      (PCB_NIGHTLY_SWEEP_DAYS; default all; else ISO weekdays 1-7 or
#      mon..sun, comma separated); --no-sweep, --no-designer and --quick
#      leave them out tonight.
#   3. Writes build/nightly/<date>/: each stage's output directory, its
#      log (<stage>.log, the rows) and stderr (<stage>.err), build.log,
#      gap.txt (compare.py --gap --targets), diff-<stage>.txt (compare.py
#      against the previous nightly that ran the stage), nightly.json and
#      summary.md (scripts/nightly-report.py --summarize). Nights beyond
#      the newest 14 keep their logs and summaries; their stages' board
#      directories are removed.
#   4. Refreshes the estimates (tier_estimates.py --refresh): the night's
#      sweep, designer, short and expansion rows join the rows of
#      build/nightly/estimates.json and the committed sidecar, the newest
#      three sources per board and kind count, and the result is
#      build/nightly/estimates.json (the next night's stages read it), so
#      that the estimates follow the binary; build/nightly/<date>/
#      estimates.log says what changed. The committed sidecar changes only
#      by hand (tier_estimates.py --refresh after a sweep, committed).
#   5. Publishes the night (since 2026-10-27 by default; --no-commit-report
#      or PCB_NIGHTLY_COMMIT_REPORT=0 turns it off): scripts/nightly-report.py
#      --commit writes the night into the main checkout's docs, as the
#      hand-written sweep write-ups v19 to v21 did, and commits them on
#      main ("nightly <date>: layout sweep vN, ...") and pushes main:
#        docs/benchmarks.md  a section "Layout sweep vN (<date>)" (vN one
#                            more than the newest numbered sweep): the
#                            totals against the previous night and the
#                            previous numbered sweep, the strict passes
#                            gained and lost with their findings, the
#                            designer tier's gap split, the costliest
#                            placements, the losers beyond the seed
#                            noise; and the night's row of "Nightly"
#        README.md           the headline's measured part and the two
#                            newest rows of the table
#        docs/handover.md    the title's state, the nightly state line,
#                            the night note
#        docs/changelog.md   "Measured on <date> (nightly: layout sweep
#                            vN, matched designer tier)"
#      The guard: only a night on which every stage ran, no row of any
#      stage was cut by the wall clock or killed, and the sweep has at
#      least 100 rows with a result is published; otherwise the night is
#      summarised only and nightly.log says why (the summary's last line).
#      The race with the supervisor's merges in the main checkout: the
#      report commits only when the main checkout is on main, its working
#      tree has no change to a tracked file and `git pull --ff-only origin
#      main` succeeds, and then commits the four files alone; otherwise it
#      writes nothing, says so in nightly.log, and the next night publishes
#      this night with its own (every guarded night after the newest one
#      docs/benchmarks.md has, at most three).
#
# One night at a time: build/nightly/.lock (a second start exits at once).
#
# For tests (scripts/test_nightly.py), environment variables replace the
# expensive parts:
#   PCB_NIGHTLY_RUN_PY       the run.py to call (a stub)
#   PCB_NIGHTLY_FAKE_BINARY  a file to snapshot instead of building (no git,
#                            no cargo; the commit is PCB_NIGHTLY_SHA)
#   PCB_NIGHTLY_ROOT         where build/ lies (default: the main checkout)
#   PCB_NIGHTLY_DATE         the night's name (default: today, YYYY-MM-DD)
#   PCB_NIGHTLY_SOURCE       the checkout to build and run from
#   PCB_NIGHTLY_CHECKOUT     the checkout the report commits into (default:
#                            the main checkout; a run with
#                            PCB_NIGHTLY_FAKE_BINARY commits only into this)
#   PCB_NIGHTLY_MIN_RESULTS  the guard's least number of sweep rows with a
#                            result (default 100)

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  sed -n '3,13p' "$0" | sed 's/^# \{0,1\}//'
  echo "options: --quick --no-sweep --no-designer --sweep-days DAYS --force --no-commit-report --install --print-units"
}

sweep=1
designer=1
force=0
commit_report="${PCB_NIGHTLY_COMMIT_REPORT:-1}"
sweep_days="${PCB_NIGHTLY_SWEEP_DAYS:-all}"
action=run
while [ $# -gt 0 ]; do
  case "$1" in
    --quick) sweep=0; designer=0 ;;
    --no-sweep) sweep=0 ;;
    --no-designer) designer=0 ;;
    --sweep-days) shift; sweep_days="${1:?--sweep-days needs a value}" ;;
    --force) force=1 ;;
    --commit-report) commit_report=1 ;;
    --no-commit-report) commit_report=0 ;;
    --install) action=install ;;
    --print-units) action=print-units ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

# The main checkout: the first entry of the worktree list, wherever this
# script runs from.
main="$(git -C "$here" worktree list --porcelain 2> /dev/null \
  | awk '/^worktree / && !found { print substr($0, 10); found = 1 }')"
main="${main:-$here}"

units() {
  # The templates with the main checkout and the installer's PATH (cargo,
  # kicad-cli and python3 must be found by the service).
  local file
  for file in nightly.service nightly.timer; do
    sed -e "s|@ROOT@|$main|g" -e "s|@PATH@|$PATH|g" "$here/scripts/$file"
  done
}

if [ "$action" = print-units ]; then
  echo "# ~/.config/systemd/user/pcb-maker-nightly.service and .timer:"
  units
  exit 0
fi

if [ "$action" = install ]; then
  if systemctl --user show-environment > /dev/null 2>&1; then
    directory="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
    mkdir -p "$directory"
    sed -e "s|@ROOT@|$main|g" -e "s|@PATH@|$PATH|g" "$here/scripts/nightly.service" \
      > "$directory/pcb-maker-nightly.service"
    sed -e "s|@ROOT@|$main|g" -e "s|@PATH@|$PATH|g" "$here/scripts/nightly.timer" \
      > "$directory/pcb-maker-nightly.timer"
    systemctl --user daemon-reload
    systemctl --user enable --now pcb-maker-nightly.timer
    systemctl --user list-timers pcb-maker-nightly.timer --no-pager
    echo "installed $directory/pcb-maker-nightly.{service,timer} (02:00 local)"
    if [ "$(loginctl show-user "$USER" --property=Linger --value 2> /dev/null)" != yes ]; then
      echo "note: without lingering the timer runs only while $USER is logged in: loginctl enable-linger $USER"
    fi
    echo "logs: journalctl --user -u pcb-maker-nightly; results: $main/build/nightly/<date>/summary.md"
  else
    echo "systemctl --user is not available here; add this line with crontab -e:"
    echo "0 2 * * * PATH=$PATH bash $main/scripts/nightly.sh >> $main/build/nightly/cron.log 2>&1"
  fi
  exit 0
fi

# --- tonight -----------------------------------------------------------------

fake_binary="${PCB_NIGHTLY_FAKE_BINARY:-}"
root="${PCB_NIGHTLY_ROOT:-$main}"
night="${PCB_NIGHTLY_DATE:-$(date +%F)}"
nights="$root/build/nightly"
bin="$root/build/bin"
mkdir -p "$nights" "$bin"

exec 9> "$nights/.lock"
if ! flock -n 9; then
  echo "another nightly holds $nights/.lock; not starting" >&2
  exit 1
fi

# The sweep's days.
weekday="$(date -d "${PCB_NIGHTLY_DATE:-today}" +%u 2> /dev/null || date +%u)"
sweep_today=0
if [ "$sweep_days" = all ]; then
  sweep_today=1
else
  names=(x mon tue wed thu fri sat sun)
  for day in ${sweep_days//,/ }; do
    if [ "$day" = "$weekday" ] || [ "$day" = "${names[$weekday]}" ]; then sweep_today=1; fi
  done
fi
[ "$sweep_today" = 1 ] || { sweep=0; designer=0; }

if [ -n "$fake_binary" ]; then
  source_dir="${PCB_NIGHTLY_SOURCE:-$here}"
  sha="${PCB_NIGHTLY_SHA:-0000000000000000000000000000000000000000}"
else
  source_dir="${PCB_NIGHTLY_SOURCE:-$(dirname "$main")/pcb-maker-nightly}"
  if [ -n "$(git -C "$main" status --porcelain --untracked-files=no)" ]; then
    echo "the main checkout $main has uncommitted changes; the nightly measures committed main only:" >&2
    git -C "$main" status --short --untracked-files=no >&2
    exit 1
  fi
  sha="$(git -C "$main" rev-parse main)"
fi
short="${sha:0:12}"

planned=(fingerprints expansion short)
[ "$sweep" = 1 ] && planned+=(sweep)
[ "$designer" = 1 ] && planned+=(designer)

# Main unchanged since a nightly that ran every stage planned tonight:
# the rows would be the same (a layout is deterministic for its binary).
if [ "$force" = 0 ]; then
  previous="$(ls -1 "$nights" 2> /dev/null | grep -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}' | sort | tail -1 || true)"
  if [ -n "$previous" ] && [ -f "$nights/$previous/nightly.json" ] \
     && python3 - "$nights/$previous/nightly.json" "$sha" "${planned[@]}" << 'PY'
import json, sys
night = json.load(open(sys.argv[1]))
ran = {name for name, stage in night.get("stages", {}).items() if stage.get("status") == "ran"}
sys.exit(0 if night.get("sha") == sys.argv[2] and set(sys.argv[3:]) <= ran else 1)
PY
  then
    echo "main $short was measured by the nightly $previous with ${planned[*]}; nothing to do (--force runs anyway)"
    exit 0
  fi
fi

directory="$nights/$night"
if [ -e "$directory" ]; then
  directory="$nights/${night}T$(date +%H%M)"
  night="$(basename "$directory")"
fi
mkdir -p "$directory"
exec > >(tee -a "$directory/nightly.log") 2>&1
started=$(date +%s)
echo "nightly $night: main $short, stages ${planned[*]}, from $source_dir"

# --- build and snapshot --------------------------------------------------------

snapshot="$bin/pcb-maker-nightly-${night:0:10}-$short"
if [ -n "$fake_binary" ]; then
  built="$fake_binary"
  echo "fake build: $fake_binary" > "$directory/build.log"
else
  if [ ! -e "$source_dir/.git" ]; then
    git -C "$main" worktree add --detach "$source_dir" "$sha"
    for path in benchmarks/real/external build artifacts benchmarks/expansion/data; do
      if [ -e "$main/$path" ] && [ ! -e "$source_dir/$path" ]; then
        mkdir -p "$(dirname "$source_dir/$path")"
        ln -s "$main/$path" "$source_dir/$path"
      fi
    done
  fi
  git -C "$source_dir" checkout --quiet --detach --force "$sha"
  echo "building $short in $source_dir"
  if ! (cd "$source_dir" && cargo build --release) > "$directory/build.log" 2>&1; then
    echo "the build failed (build.log):" >&2
    tail -20 "$directory/build.log" >&2
    exit 1
  fi
  built="$source_dir/target/release/pcb-maker"
fi
if [ -e "$snapshot" ]; then
  cmp -s "$built" "$snapshot" || { echo "$snapshot exists and is another binary" >&2; exit 1; }
else
  cp "$built" "$snapshot"
fi
echo "snapshot $snapshot"

# The newest 7 nightly snapshots stay (by name: the date comes first);
# one a running process uses stays too.
in_use() {
  local exe
  for exe in /proc/[0-9]*/exe; do
    [ "$(readlink "$exe" 2> /dev/null)" = "$1" ] && return 0
  done
  return 1
}
mapfile -t old < <(ls -1 "$bin" | grep -E '^pcb-maker-nightly-' | sort -r | tail -n +8)
for name in ${old[@]+"${old[@]}"}; do
  if in_use "$bin/$name"; then echo "kept $name (in use)"; else rm -f "$bin/$name"; echo "removed $name"; fi
done

# --- the stages ----------------------------------------------------------------

run_py="${PCB_NIGHTLY_RUN_PY:-$source_dir/benchmarks/agent-tasks/run.py}"
tasks="$source_dir/benchmarks/agent-tasks/tasks/v7/tasks.json"
common=(--binary "$snapshot" --ledger-priority nightly)
# The nightly's own estimates (refreshed at the end of each night, below),
# else run.py's default, the committed sidecar.
estimates="$nights/estimates.json"
committed_estimates="$source_dir/benchmarks/agent-tasks/estimates.json"
if [ -f "$estimates" ]; then common+=(--estimates "$estimates"); fi
statuses=()

stage() {
  # stage <name> [ENV=VALUE ...] -- <run.py arguments>
  local name="$1" begin status
  shift
  local environment=()
  while [ "$1" != -- ]; do environment+=("$1"); shift; done
  shift
  begin=$(date +%s)
  echo "== $name: $(date +%T)"
  set +e
  (cd "$source_dir" && env ${environment[@]+"${environment[@]}"} python3 "$run_py" "$directory/$name" "$@" \
    "${common[@]}") > "$directory/$name.log" 2> "$directory/$name.err"
  status=$?
  set -e
  statuses+=("$name=$status")
  echo "== $name: exit $status, $(($(date +%s) - begin)) s; $(grep -E 'tasks pass' "$directory/$name.log" | tail -1)"
}

stage fingerprints PCB_ROUTER_FINGERPRINT=1 PCB_ROUTER_NO_WALL_GUARD=1 -- \
  --tasks "$tasks" --route-from "$root/build/github-layout-v16" --budget-scale 0.25 --jobs 1 --threads 4 \
  --only wntrblm__Castor_and_Pollux__mainboard transistorfet__computie__k30-SBC \
  antmicro__jetson-nano-baseboard__jetson-nano-baseboard briskspirit__Sisu_SSE-9__Sisu_SSE-9
stage expansion -- --tier expansion --jobs 8 --threads 1
stage short -- --tier short --jobs 3 --threads 2
if [ "$designer" = 1 ]; then
  # The matched designer tier: the median final-ladder share of the
  # newest earlier night's sweep (README.md, "The designer tier"; the
  # designer stage runs before tonight's sweep); 0.695 (v18's) without a
  # sweep row that records it.
  last_sweep="$(ls -1d "$nights"/[0-9]*/sweep.log 2> /dev/null | grep -v "^$directory/" | sort | tail -1 || true)"
  scale="$(python3 "$here/scripts/nightly-report.py" --ladder-share "${last_sweep:-/nonexistent}" --default 0.695)"
  echo "designer budget scale $scale (${last_sweep:-no earlier sweep})"
  stage designer -- --tier designer --budget-scale "$scale" --quality --jobs 8 --threads 1 --order peak-desc
fi
if [ "$sweep" = 1 ]; then
  stage sweep -- --tasks "$tasks" --jobs 10 --threads 2 --quality --order peak-desc
fi

# --- the summary -----------------------------------------------------------------

python3 "$here/scripts/nightly-report.py" --summarize "$directory" --nights "$nights" --sha "$sha" \
  --binary "$snapshot" --harness "$source_dir" --planned "${planned[@]}" --statuses "${statuses[@]}" \
  --seconds "$(($(date +%s) - started))" ${PCB_NIGHTLY_MIN_RESULTS:+--min-results "$PCB_NIGHTLY_MIN_RESULTS"}
echo "summary $directory/summary.md"

# The estimates follow the binary: tonight's rows into the nightly's own
# sidecar (from the committed one on the first night).
base="$estimates"
[ -f "$base" ] || base="$committed_estimates"
if python3 "$source_dir/benchmarks/agent-tasks/tier_estimates.py" --refresh --night "$directory" \
     --estimates "$base" --merge "$committed_estimates" --output "$estimates" > "$directory/estimates.log" 2>&1; then
  echo "estimates $estimates: $(head -1 "$directory/estimates.log" | sed 's/^[^:]*: //')"
else
  echo "the estimates' refresh failed (estimates.log):" >&2
  tail -5 "$directory/estimates.log" >&2
fi

# Nights beyond the newest 14 keep their logs and summaries, not their
# board directories (a night with the sweep and the designer tier holds
# about 2 GB).
mapfile -t stale < <(ls -1 "$nights" | grep -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}' | sort -r | tail -n +15)
for name in ${stale[@]+"${stale[@]}"}; do
  for part in fingerprints expansion short sweep designer; do
    [ -d "$nights/$name/$part" ] && rm -rf "${nights:?}/$name/$part"
  done
done

if [ "$commit_report" = 1 ]; then
  if [ -n "$fake_binary" ] && [ -z "${PCB_NIGHTLY_CHECKOUT:-}" ]; then
    echo "report not committed: a test run (PCB_NIGHTLY_FAKE_BINARY) commits only into PCB_NIGHTLY_CHECKOUT"
  elif ! python3 "$here/scripts/nightly-report.py" --nights "$nights" --commit \
         --checkout "${PCB_NIGHTLY_CHECKOUT:-$main}" ${PCB_NIGHTLY_MIN_RESULTS:+--min-results "$PCB_NIGHTLY_MIN_RESULTS"}; then
    echo "the report failed; nothing committed"
  fi
else
  echo "report not committed (--no-commit-report or PCB_NIGHTLY_COMMIT_REPORT=0)"
fi

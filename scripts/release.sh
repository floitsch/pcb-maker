#!/usr/bin/env bash
# Copyright (C) 2026 Toit contributors.
#
# The rolling `snapshot` pre-release
# (https://github.com/floitsch/pcb-maker/releases/tag/snapshot): what
# .github/workflows/release.yml does on every push to main, by hand for a
# maintainer with `gh` installed and logged in.
#
#   bash scripts/release.sh             # build, check, package and publish HEAD
#   bash scripts/release.sh --dry-run   # the same without publishing (dist/ only)
#
# The steps, also called one by one by the workflow:
#
#   bash scripts/release.sh package <platform> <binary> <dist>
#       writes <dist>/pcb-maker-<platform>.tar.gz (the binary, README.md,
#       LICENSE, VERSION, docs/agents.md, skills/pcb-maker/SKILL.md) and
#       <dist>/VERSION (the commit, its date, the build date)
#   bash scripts/release.sh publish <dist>
#       deletes the `snapshot` release and its tag and creates them again at
#       HEAD with every archive in <dist>, VERSION and SHA256SUMS
#
# Publishing replaces the whole release: a local run on Linux publishes
# linux-x86_64 only (the next push to main brings macos-aarch64 back).
# HEAD must be clean and on origin (the tag is made on GitHub's copy).

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
tag=snapshot

usage() {
  echo "usage: $0 [--dry-run] | package <platform> <binary> <dist> | publish <dist>" >&2
  exit 2
}

# Writes <dist>/VERSION for HEAD.
version_file() {
  local dist="$1"
  mkdir -p "$dist"
  {
    echo "commit $(git rev-parse HEAD)"
    echo "committed $(git log -1 --format=%cI HEAD)"
    echo "built $(date -u +%Y-%m-%d)"
  } > "$dist/VERSION"
}

package() {
  local platform="$1" binary="$2" dist="$3"
  local stage="pcb-maker-$platform"
  local work
  version_file "$dist"
  work="$(mktemp -d)"
  mkdir -p "$work/$stage/docs" "$work/$stage/skills/pcb-maker"
  cp "$binary" "$work/$stage/pcb-maker"
  cp README.md LICENSE "$dist/VERSION" "$work/$stage/"
  cp docs/agents.md "$work/$stage/docs/"
  cp skills/pcb-maker/SKILL.md "$work/$stage/skills/pcb-maker/"
  tar czf "$dist/$stage.tar.gz" -C "$work" "$stage"
  rm -r "$work"
  echo "$dist/$stage.tar.gz"
}

publish() {
  local dist="$1"
  local sha short notes
  sha="$(git rev-parse HEAD)"
  short="$(git rev-parse --short HEAD)"
  shopt -s nullglob
  local archives=("$dist"/pcb-maker-*.tar.gz)
  shopt -u nullglob
  if [ ${#archives[@]} -eq 0 ]; then
    echo "no archives in $dist" >&2
    exit 1
  fi
  [ -f "$dist/VERSION" ] || version_file "$dist"
  local sum=(sha256sum)
  command -v sha256sum > /dev/null || sum=(shasum -a 256)
  (cd "$dist" && "${sum[@]}" pcb-maker-*.tar.gz VERSION > SHA256SUMS)
  notes="Automatic build of \`$sha\` ($(git log -1 --format=%cs HEAD)), the latest commit on \`main\` that passed \`scripts/check.sh --fast\`.

Archives: $(cd "$dist" && ls pcb-maker-*.tar.gz | sed 's/^/`/; s/$/`/' | paste -sd, - | sed 's/,/, /g'). Each holds the \`pcb-maker\` binary, README.md, LICENSE, VERSION, docs/agents.md and skills/pcb-maker/SKILL.md. KiCad 10 (\`kicad-cli\`) must be on the PATH.

This is a moving pre-release: every push to \`main\` replaces it. To pin a version, keep the archive and record the commit in \`VERSION\` (\`pcb-maker --version\` prints it), or build that commit from source (\`git checkout <commit> && cargo build --release\`)."
  # Delete and recreate: no asset of an older commit survives.
  gh release delete "$tag" --cleanup-tag --yes 2> /dev/null || true
  gh release create "$tag" --prerelease --target "$sha" --title "Snapshot $short" --notes "$notes" \
    "${archives[@]}" "$dist/VERSION" "$dist/SHA256SUMS"
}

# The host's platform name, as the archives call it.
host_platform() {
  local os arch
  case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    *) echo "unsupported system $(uname -s)" >&2; exit 1 ;;
  esac
  case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) echo "unsupported machine $(uname -m)" >&2; exit 1 ;;
  esac
  echo "$os-$arch"
}

case "${1:-}" in
  package) [ $# -eq 4 ] || usage; package "$2" "$3" "$4"; exit 0 ;;
  publish) [ $# -eq 2 ] || usage; publish "$2"; exit 0 ;;
  --dry-run) [ $# -eq 1 ] || usage; dry_run=1 ;;
  "") dry_run=0 ;;
  *) usage ;;
esac

if ! git diff --quiet HEAD --; then
  echo "the checkout has uncommitted changes; a release is built from a commit" >&2
  exit 1
fi
if [ "$dry_run" = 0 ]; then
  command -v gh > /dev/null || { echo "gh (the GitHub CLI) is not installed" >&2; exit 1; }
  git fetch --quiet origin
  if [ -z "$(git branch -r --contains HEAD)" ]; then
    echo "HEAD $(git rev-parse --short HEAD) is not on origin; push it first" >&2
    exit 1
  fi
fi

bash scripts/check.sh --fast
cargo build --release --locked --bin pcb-maker
dist="$root/dist"
rm -rf "$dist"
package "$(host_platform)" target/release/pcb-maker "$dist"
if [ "$dry_run" = 1 ]; then
  echo "dry run: $dist is what would be published"
  exit 0
fi
publish "$dist"

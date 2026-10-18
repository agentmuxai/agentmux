#!/usr/bin/env bash
# Extract the changelog block for a given version from VERSION_HISTORY.md.
# Usage: bash scripts/extract-changelog.sh [--include-unpublished] [version]
# If version is omitted, reads it from package.json.
# Outputs the markdown body between "## <version>" and the next "## " heading.
# Exits 1 if the version entry is missing — never silently ships empty notes.
#
# The notes also carry the oldest version a release can upgrade from in place
# (UPGRADE_FLOOR, or $UPGRADE_FLOOR_FILE), as a hidden line srv's update check
# reads: `<!-- agentmux:min-upgrade-from=0.55.0 -->`
# (docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md §3).
#
# --include-unpublished: also append each older version that has no GitHub
# release, newest first, stopping at the first one that has. A release whose
# build failed is never published (release.yml's publish needs every build), so
# its changes ship in the next release; without this its notes appear on no
# release page (v0.59.29, whose changes shipped in v0.59.30). Asks
# `gh release view` against $GITHUB_REPOSITORY (default agentmuxai/agentmux).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HISTORY="${VERSION_HISTORY_FILE:-$REPO_ROOT/VERSION_HISTORY.md}"

INCLUDE_UNPUBLISHED=0
if [ "${1:-}" = "--include-unpublished" ]; then
  INCLUDE_UNPUBLISHED=1
  shift
fi

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  VERSION=$(node -p "require('$REPO_ROOT/package.json').version")
fi

# The entries of one version's section, blank lines dropped.
section() {
  awk -v v="$1" '$0 ~ "^## " && ($2 == v) {found=1; next} found && /^## /{exit} found{print}' \
    "$HISTORY" | sed '/^[[:space:]]*$/d'
}

NOTES=$(section "$VERSION")

if [ -z "$NOTES" ]; then
  echo "❌ extract-changelog: no entry for v${VERSION} in VERSION_HISTORY.md — run 'task release' before tagging" >&2
  exit 1
fi

echo "$NOTES"

FLOOR_FILE="${UPGRADE_FLOOR_FILE:-$REPO_ROOT/UPGRADE_FLOOR}"
if [ -f "$FLOOR_FILE" ]; then
  FLOOR=$(tr -d '[:space:]' < "$FLOOR_FILE")
  if ! [[ "$FLOOR" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "❌ extract-changelog: $FLOOR_FILE must hold one version (major.minor.patch), not '${FLOOR}'" >&2
    exit 1
  fi
  echo ""
  echo "<!-- agentmux:min-upgrade-from=${FLOOR} -->"
fi

[ "$INCLUDE_UNPUBLISHED" = 1 ] || exit 0

REPO="${GITHUB_REPOSITORY:-agentmuxai/agentmux}"
# The versions after $VERSION in file order (newest first), at most 10.
OLDER=$(awk -v v="$VERSION" '/^## /{if (seen) print $2; if ($2 == v) seen=1}' "$HISTORY" | head -10)
for older in $OLDER; do
  if err=$(gh release view "v${older}" --repo "$REPO" 2>&1 > /dev/null); then
    break
  fi
  # Only a missing release counts; any other failure (auth, network) stops
  # here rather than listing every older version as unpublished.
  if ! grep -q "release not found" <<< "$err"; then
    echo "⚠️ extract-changelog: couldn't check v${older} (${err}); notes stop at v${VERSION}" >&2
    break
  fi
  echo ""
  echo "### Also in this release: v${older}, which was never published"
  echo ""
  section "$older"
done

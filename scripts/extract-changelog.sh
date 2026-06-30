#!/usr/bin/env bash
# Extract the changelog block for a given version from VERSION_HISTORY.md.
# Usage: bash scripts/extract-changelog.sh [version]
# If version is omitted, reads it from package.json.
# Outputs the markdown body between "## <version>" and the next "## " heading.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  VERSION=$(node -p "require('$REPO_ROOT/package.json').version")
fi

awk "/^## ${VERSION}([[:space:]]|$)/{found=1; next} found && /^## /{exit} found{print}" \
  "$REPO_ROOT/VERSION_HISTORY.md" | sed '/^[[:space:]]*$/d; 1s/^[[:space:]]*//'

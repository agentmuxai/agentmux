#!/usr/bin/env bash
# changeset.sh — author a new changeset file.
#
# Usage:
#   scripts/changeset.sh <type> "<description>"
#
# Example:
#   scripts/changeset.sh patch "fix(auth): cancel in-flight session on selection swap"
#
# Allowed types: patch | minor | major
#
# Produces:
#   .changesets/<unix-ts>-<slug>.md
#
# The file's frontmatter holds the bump type; the body is the description.
# RFC #857 Phase 2 / spec docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md.

set -euo pipefail

TYPE="${1:-}"
DESC="${2:-}"

if [[ -z "$TYPE" || -z "$DESC" ]]; then
    cat >&2 <<EOF
Usage: $0 <patch|minor|major> "<description>"

Example:
    $0 patch "fix(auth): cancel in-flight session on selection swap"
EOF
    exit 1
fi

case "$TYPE" in
    patch|minor|major) ;;
    *)
        echo "ERROR: type must be one of: patch | minor | major (got: $TYPE)" >&2
        exit 1
        ;;
esac

# Locate repo root (the script may be invoked from anywhere).
REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
    echo "ERROR: not inside a git repository." >&2
    exit 1
fi

DIR="$REPO_ROOT/.changesets"
mkdir -p "$DIR"

# Build a filename: <unix-ts>-<slug-of-description>.md
# Slug: lowercase, replace anything non-alphanumeric with `-`, collapse, trim.
TS="$(date +%s)"
SLUG="$(printf '%s' "$DESC" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -E 's/[^a-z0-9]+/-/g; s/^-+//; s/-+$//' \
    | cut -c1-60)"
[[ -z "$SLUG" ]] && SLUG="change"

FILE="$DIR/${TS}-${SLUG}.md"

cat >"$FILE" <<EOF
---
type: $TYPE
---

$DESC
EOF

echo "Wrote $FILE" >&2
echo "$FILE"

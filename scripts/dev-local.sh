#!/usr/bin/env bash
# dev-local.sh — run `task dev` with a temporarily-bumped version label,
# then restore the original version files on exit.
#
# Use case: under the RFC #857 changeset workflow, feature PRs no longer
# bump `package.json` / `Cargo.toml`. The dev build's version label stays
# pinned to the last release PR's version across many merges, which:
#   - makes it hard to tell "which merge does this dev build correspond
#     to" from the version alone, and
#   - lets cargo's incremental cache serve stale `.o` files because
#     CARGO_PKG_VERSION doesn't change across `task dev` invocations.
#
# This script mirrors `scripts/package-local.sh`: bump in-place, run
# `task dev`, restore working-tree on exit (Ctrl+C, dev-loop crash,
# anything). No git mutation, no commit.
#
# Usage:
#   scripts/dev-local.sh          # patch bump (default)
#   scripts/dev-local.sh patch    # explicit
#   scripts/dev-local.sh minor
#   scripts/dev-local.sh major
#
# RFC #857 Phase 2 escape hatch — see .changesets/README.md.

set -euo pipefail

BUMP_TYPE="${1:-patch}"

case "$BUMP_TYPE" in
    patch|minor|major) ;;
    *)
        echo "ERROR: bump type must be patch | minor | major (got: $BUMP_TYPE)" >&2
        exit 1
        ;;
esac

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

ORIG_VERSION="$(node -p "require('./package.json').version")"
echo ">>> dev-local: starting at v$ORIG_VERSION" >&2

# Capture working-tree state of files bump-cli will rewrite, so we can
# restore them byte-for-byte regardless of whether bump-cli leaves staged
# changes behind.
BACKUP_DIR="$(mktemp -d)"

VERSION_FILES=(
    package.json
    package-lock.json
    Cargo.lock
    Cargo.toml
)
for f in "${VERSION_FILES[@]}"; do
    if [[ -f "$f" ]]; then
        cp "$f" "$BACKUP_DIR/$(basename "$f").bak"
    fi
done

# Restore on exit no matter what (Ctrl+C, dev-loop crash, etc).
# restore_files MUST run before BACKUP_DIR is deleted — the trap below
# runs restore first, then cleans up the backups.
restore_files() {
    echo >&2
    echo ">>> dev-local: restoring working-tree to v$ORIG_VERSION" >&2
    for f in "${VERSION_FILES[@]}"; do
        if [[ -f "$BACKUP_DIR/$(basename "$f").bak" ]]; then
            cp "$BACKUP_DIR/$(basename "$f").bak" "$f"
        fi
    done
    # Drop any staging bump-cli did.
    git reset -q -- "${VERSION_FILES[@]}" 2>/dev/null || true
    echo ">>> dev-local: working tree restored." >&2
}
trap 'restore_files; rm -rf "$BACKUP_DIR"' EXIT

# Bump in-place (no --commit). bump-cli handles Cargo workspace inheritance
# via .bump.json (Phase 1 collapsed 5 member Cargo.toml targets to 1 root).
bump "$BUMP_TYPE"
NEW_VERSION="$(node -p "require('./package.json').version")"
echo ">>> dev-local: temporarily bumped to v$NEW_VERSION for dev session" >&2
echo ">>> dev-local: launching task dev — Ctrl+C to stop, restore is automatic" >&2

# Run the actual dev loop. It blocks until the user kills it; the EXIT
# trap then fires `restore_files`.
task dev

echo >&2
echo ">>> dev-local: dev loop exited." >&2
echo ">>> Working tree is back at v$ORIG_VERSION — no git mutation." >&2

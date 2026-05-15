#!/usr/bin/env bash
# package-local.sh — produce a labeled portable build by temporarily bumping
# the version, packaging, then restoring the original version files.
#
# Use case: comparing two builds side-by-side on Desktop. `task package`
# names artifacts `agentmux-<version>-x64-portable.zip`; if the version
# doesn't change between builds, the new ZIP overwrites the old. This script
# bumps once, builds once, and restores so your working tree is clean
# afterward — no `git reset` dance, no accidentally-pushed bump commits.
#
# Usage:
#   scripts/package-local.sh          # patch bump (default)
#   scripts/package-local.sh patch    # explicit
#   scripts/package-local.sh minor
#   scripts/package-local.sh major
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
echo ">>> package-local: starting at v$ORIG_VERSION" >&2

# Capture working-tree state of files bump-cli will rewrite, so we can
# restore them byte-for-byte regardless of whether bump-cli leaves staged
# changes behind.
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

VERSION_FILES=(
    package.json
    package-lock.json
    Cargo.lock
    Cargo.toml
)
for f in "${VERSION_FILES[@]}"; do
    if [[ -f "$f" ]]; then
        cp "$f" "$TMPDIR/$(basename "$f").bak"
    fi
done

# Bump in-place (no --commit). bump-cli handles Cargo workspace inheritance
# via .bump.json (Phase 1 collapsed 5 member Cargo.toml targets to 1 root).
bump "$BUMP_TYPE"
NEW_VERSION="$(node -p "require('./package.json').version")"
echo ">>> package-local: temporarily bumped to v$NEW_VERSION for build" >&2

# Restore on exit no matter what (build failure, Ctrl-C, etc).
restore_files() {
    echo >&2
    echo ">>> package-local: restoring working-tree to v$ORIG_VERSION" >&2
    for f in "${VERSION_FILES[@]}"; do
        if [[ -f "$TMPDIR/$(basename "$f").bak" ]]; then
            cp "$TMPDIR/$(basename "$f").bak" "$f"
        fi
    done
    # Drop any staging bump-cli did.
    git reset -q -- "${VERSION_FILES[@]}" 2>/dev/null || true
    echo ">>> package-local: working tree restored." >&2
}
trap 'rm -rf "$TMPDIR"; restore_files' EXIT

# Run the actual package build.
task package

echo >&2
echo ">>> package-local: build complete." >&2
echo ">>> Artifact at: ~/Desktop/agentmux-${NEW_VERSION}-x64-portable.zip" >&2
echo ">>> Working tree is back at v$ORIG_VERSION — no git mutation." >&2

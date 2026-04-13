#!/usr/bin/env bash
# bump-wrapper.sh — wraps @a5af/bump-cli so package-lock.json actually gets synced.
#
# Background: bump-cli's "npm" lockfile strategy runs `npm version` under the
# hood, which fails with "Unknown error" while bump-cli holds the working tree
# in a partial state. Because `allowFailure: true` in the default config,
# bump-cli silently proceeds, leaving package-lock.json pinned to the previous
# version. PR #341 caught the resulting inconsistency in review.
#
# This wrapper runs bump normally, then syncs the lockfile with a direct
# `npm install --package-lock-only` call, and (if bump created a commit) folds
# the synced lockfile into the same commit via `git commit --amend`. That
# single amend is deliberate — it produces a clean, consistent bump commit
# without leaving an orphaned "lockfile sync" commit trailing behind every
# version bump.
#
# Usage: mirrors bump-cli exactly.
#   scripts/bump-wrapper.sh patch -m "description" --commit
#   scripts/bump-wrapper.sh minor --commit
#   scripts/bump-wrapper.sh 1.2.3 --commit
#
# Exit codes match bump-cli: non-zero on any failure, including the lockfile
# sync. If --commit is not passed, the wrapper skips the amend step and just
# syncs the lockfile in-place (caller is responsible for staging).

set -euo pipefail

# Pass everything through to the real bump CLI.
if ! command -v bump >/dev/null 2>&1; then
    echo "ERROR: bump-cli not installed. Install with: npm install -g @a5af/bump-cli" >&2
    exit 1
fi

# Detect whether --commit was requested — controls whether we amend.
DO_COMMIT=0
for arg in "$@"; do
    if [ "$arg" = "--commit" ]; then
        DO_COMMIT=1
    fi
done

# Remember HEAD so we can detect whether bump actually created a commit.
BEFORE_HEAD=""
if [ "$DO_COMMIT" -eq 1 ]; then
    BEFORE_HEAD=$(git rev-parse HEAD 2>/dev/null || echo "")
fi

# Run the real bump CLI.
bump "$@"

# Sync package-lock.json directly. --ignore-scripts keeps lifecycle hooks from
# running during what should be a pure metadata operation.
echo ""
echo "bump-wrapper: syncing package-lock.json …"
npm install --package-lock-only --ignore-scripts >/dev/null 2>&1 || {
    echo "ERROR: npm install --package-lock-only failed; lockfile is out of sync" >&2
    exit 1
}

# Verify the versions now match, just in case.
PKG_VER=$(node -p "require('./package.json').version")
LOCK_VER=$(node -p "require('./package-lock.json').version")
if [ "$PKG_VER" != "$LOCK_VER" ]; then
    echo "ERROR: version mismatch after sync: package.json=$PKG_VER lockfile=$LOCK_VER" >&2
    exit 1
fi
echo "bump-wrapper: package-lock.json synced to $LOCK_VER"

# If bump committed, amend the lockfile into that same commit.
if [ "$DO_COMMIT" -eq 1 ]; then
    AFTER_HEAD=$(git rev-parse HEAD 2>/dev/null || echo "")
    if [ -n "$BEFORE_HEAD" ] && [ "$BEFORE_HEAD" != "$AFTER_HEAD" ]; then
        if ! git diff --quiet package-lock.json 2>/dev/null; then
            git add package-lock.json
            git commit --amend --no-edit >/dev/null
            echo "bump-wrapper: folded lockfile into bump commit $(git rev-parse --short HEAD)"
        fi
    fi
fi

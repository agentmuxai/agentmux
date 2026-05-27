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
# Detect whether this invocation came from `task package` (portable
# build bump). bump-wrapper.sh is also called by `scripts/release.sh`
# for the real release pipeline; release.sh handles VERSION_HISTORY
# itself, so we only want to auto-append for the task-package path.
# Signal: `-m "build"` — the exact message string `task package`
# passes (see Taskfile.yml `package:`). release.sh uses a no-message
# bump and a later explicit commit.
IS_PORTABLE_BUMP=0
prev_arg=""
for arg in "$@"; do
    if [ "$arg" = "--commit" ]; then
        DO_COMMIT=1
    fi
    if [ "$prev_arg" = "-m" ] && [ "$arg" = "build" ]; then
        IS_PORTABLE_BUMP=1
    fi
    prev_arg="$arg"
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

# Portable-bump auto-append: when invoked from `task package`, also
# fold a no-semantic-content VERSION_HISTORY.md entry into the same
# bump commit. Reason: `task package` is a build-counter increment,
# not a release — but reagent's release-consistency invariant
# requires VERSION_HISTORY.md's top heading to match package.json.
# Without this, every PR opened after a `task package` run gets a
# reagent P0 about the drift, and authors hand-backfill the entry
# (see e.g. PRs #1057 / #1060 / #1068 / #1072 / #1075 from 2026-05-26).
#
# `task release` does NOT take this path — it calls bump-wrapper.sh
# without `-m "build"` and appends a real release entry from the
# consumed changesets. So the auto-append fires ONLY for portable
# builds, which is exactly where the drift comes from.
if [ "$DO_COMMIT" -eq 1 ] && [ "$IS_PORTABLE_BUMP" -eq 1 ]; then
    if [ -f VERSION_HISTORY.md ]; then
        NEW_HEADING="## $PKG_VER — $(date +%Y-%m-%d)"
        # Skip if a matching entry was already written for this version
        # (e.g. a hand-backfill in the same commit, or a retry).
        if ! grep -q "^## $PKG_VER " VERSION_HISTORY.md; then
            # Insert AFTER the top-of-file `# AgentMux Version History`
            # heading. Use a temp file + mv to keep the edit atomic and
            # to preserve trailing newlines / encoding.
            VH_TMP="$(mktemp)"
            # Insert the new section right after the H1. We print our
            # own blank+heading+blank+content, then a trailing blank
            # (separator), then `next` to consume the H1. The original
            # blank-after-H1 (line 2) is preserved by the catch-all
            # `{ print }`, producing two blanks between our entry and
            # the previous-top heading — which markdown collapses to
            # one blank in render, so it reads identically to a hand-
            # backfilled entry.
            awk -v heading="$NEW_HEADING" '
                NR == 1 && /^# / {
                    print
                    print ""
                    print heading
                    print ""
                    print "- (no semantic content — internal portable-build counter increment from `task package`; auto-appended by `scripts/bump-wrapper.sh` to satisfy the release-consistency invariant)"
                    print ""
                    next
                }
                { print }
            ' VERSION_HISTORY.md > "$VH_TMP"
            mv "$VH_TMP" VERSION_HISTORY.md

            git add VERSION_HISTORY.md
            git commit --amend --no-edit >/dev/null
            echo "bump-wrapper: appended VERSION_HISTORY entry for $PKG_VER (portable bump auto-fill)"
        fi
    fi
fi

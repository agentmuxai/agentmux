#!/usr/bin/env bash
# release.sh — consume pending .changesets/*.md, bump, update history, delete.
#
# Reads every `.changesets/*.md`, picks the highest bump type from their
# frontmatter (major > minor > patch), runs scripts/bump-wrapper.sh to bump
# the version (which handles both Cargo workspace + package.json + lockfiles
# via .bump.json), appends entries to VERSION_HISTORY.md, deletes the consumed
# changesets, and stages everything. The caller (a developer or a bot)
# commits the staged changes and opens a release PR.
#
# Usage:
#   scripts/release.sh [--dry-run]
#
# RFC #857 Phase 2 / spec docs/specs/SPEC_MULTI_AGENT_VERSION_COORDINATION_2026_05_15.md.

set -euo pipefail

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN=1
fi

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
    echo "ERROR: not inside a git repository." >&2
    exit 1
fi
cd "$REPO_ROOT"

shopt -s nullglob
CHANGESETS=(.changesets/[0-9]*.md)

if (( ${#CHANGESETS[@]} == 0 )); then
    echo "No pending changesets. Nothing to release." >&2
    exit 0
fi

echo "Found ${#CHANGESETS[@]} pending changeset(s):" >&2
for f in "${CHANGESETS[@]}"; do
    echo "  - ${f#.changesets/}" >&2
done
echo >&2

# Determine highest bump type. Order: major > minor > patch.
RELEASE_TYPE="patch"
for f in "${CHANGESETS[@]}"; do
    TYPE="$(awk '/^---$/{n++;next} n==1 && /^type:/{print $2; exit}' "$f" | tr -d '[:space:]')"
    case "$TYPE" in
        major) RELEASE_TYPE="major"; break ;;
        minor) [[ "$RELEASE_TYPE" != "major" ]] && RELEASE_TYPE="minor" ;;
        patch) ;;
        *)
            echo "WARNING: $f has unknown type '$TYPE', treating as patch." >&2
            ;;
    esac
done

echo "Release type: $RELEASE_TYPE" >&2

# Capture one description per changeset: the first non-empty body line (the
# changeset title). A changeset body may carry extra prose lines for reviewer
# context; those must NOT each become their own changelog bullet (issue #1200).
DESCRIPTIONS=()
for f in "${CHANGESETS[@]}"; do
    TITLE="$(awk '/^---$/{n++;next} n==2 && NF {print; exit}' "$f")"
    [[ -n "$TITLE" ]] && DESCRIPTIONS+=("$TITLE")
done

if (( DRY_RUN )); then
    echo >&2
    echo "DRY RUN — would bump $RELEASE_TYPE with these entries:" >&2
    for d in "${DESCRIPTIONS[@]}"; do
        echo "  - $d" >&2
    done
    exit 0
fi

# Bump version + sync lockfiles. The wrapper handles --commit; we run WITHOUT
# --commit so we can fold the version-history update + changeset deletes into
# the same final commit.
if [[ ! -x scripts/bump-wrapper.sh ]]; then
    echo "ERROR: scripts/bump-wrapper.sh not found or not executable." >&2
    exit 1
fi

# bump-cli reads "current" from package.json and computes "next"; we capture
# both for the history entry.
CURRENT_VERSION="$(node -p "require('./package.json').version")"
scripts/bump-wrapper.sh "$RELEASE_TYPE"
NEW_VERSION="$(node -p "require('./package.json').version")"

echo "Bumped $CURRENT_VERSION -> $NEW_VERSION" >&2

# Append to VERSION_HISTORY.md (create if missing).
HISTORY="VERSION_HISTORY.md"
if [[ ! -f "$HISTORY" ]]; then
    echo "# Version History" >"$HISTORY"
    echo >>"$HISTORY"
fi

# Insert the new entry at the top (after the # heading).
TMP="$(mktemp)"
TODAY="$(date +%Y-%m-%d)"
{
    head -n1 "$HISTORY"
    echo
    echo "## $NEW_VERSION — $TODAY"
    echo
    for d in "${DESCRIPTIONS[@]}"; do
        echo "- $d"
    done
    echo
    tail -n +2 "$HISTORY"
} >"$TMP"
mv "$TMP" "$HISTORY"

# Delete consumed changesets.
git rm -q -- "${CHANGESETS[@]}"

# Stage version history + lockfiles. bump-wrapper.sh in no-commit mode
# syncs `package-lock.json` on-disk but does NOT stage it (see its line
# 23-24); we stage explicitly here so the release commit ships consistent
# versions across package.json + package-lock.json. Reagent P1 on #865.
git add -- "$HISTORY" package-lock.json

# ── Verify the release files agree (retro 2026-05-22 action item) ─────────
#
# bump-cli has been known to silently skip a target file (e.g. when the
# description is too long). When that happens, only some of
# {package.json, Cargo.toml, lockfiles} get bumped — VERSION_HISTORY still
# advances — and the release commit ships an inconsistent set. The next
# `task release` then proposes a regressed version, and reviewers have no
# automatic signal to refuse it. This guard re-reads every version
# location on disk and fails loudly if they don't all agree.
#
# See docs/retro/retro-release-version-desync-2026-05-22.md.
INCONSISTENCIES=()

PKG_V="$(node -p "require('./package.json').version" 2>/dev/null || echo '<unreadable>')"
[[ "$PKG_V" == "$NEW_VERSION" ]] || \
    INCONSISTENCIES+=("package.json version=$PKG_V")

CARGO_V="$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version *= *"\(.*\)"$/\1/p}' Cargo.toml | head -1)"
[[ "$CARGO_V" == "$NEW_VERSION" ]] || \
    INCONSISTENCIES+=("Cargo.toml [workspace.package].version=$CARGO_V")

PL_V="$(node -p "require('./package-lock.json').version" 2>/dev/null || echo '<unreadable>')"
[[ "$PL_V" == "$NEW_VERSION" ]] || \
    INCONSISTENCIES+=("package-lock.json version=$PL_V")

CL_V="$(sed -n '/^name = "agentmux-cef"$/{n;s/^version = "\(.*\)"$/\1/p;q}' Cargo.lock)"
[[ "$CL_V" == "$NEW_VERSION" ]] || \
    INCONSISTENCIES+=("Cargo.lock agentmux-cef version=$CL_V")

VH_V="$(awk '/^## /{print $2; exit}' "$HISTORY")"
[[ "$VH_V" == "$NEW_VERSION" ]] || \
    INCONSISTENCIES+=("VERSION_HISTORY.md top=$VH_V")

if (( ${#INCONSISTENCIES[@]} > 0 )); then
    echo "" >&2
    echo "ERROR: release-consistency check failed." >&2
    echo "Expected every version location to equal '$NEW_VERSION', but found:" >&2
    for e in "${INCONSISTENCIES[@]}"; do
        echo "  - $e" >&2
    done
    echo "" >&2
    echo "This usually means bump-cli silently skipped a file. Inspect:" >&2
    echo "  git diff --staged" >&2
    echo "Then fix the mismatched files before committing." >&2
    echo "" >&2
    echo "See docs/retro/retro-release-version-desync-2026-05-22.md." >&2
    exit 1
fi

echo >&2
echo "Release prepared (NOT committed) — all 5 version locations agree at $NEW_VERSION. Review with: git diff --staged" >&2
echo "Then commit and push:" >&2
echo "  git commit -m 'chore: release v$NEW_VERSION'" >&2
echo "  git push -u origin <branch>" >&2
